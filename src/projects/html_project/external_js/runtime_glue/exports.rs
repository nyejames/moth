//! Referenced export collection for HTML JS glue.
//!
//! WHAT: identifies which externally-referenced functions use `ExternalModuleExport` lowering
//!       and produces deterministic, sorted metadata for glue generation.
//! WHY: the JS backend may reference many external functions, but only module exports need
//!      generated glue wrappers.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::{
    ExternalFunctionId, ExternalJsLowering, ExternalPackageId, ExternalPackageRegistry,
    ExternalSignatureType,
};
use crate::compiler_frontend::module_compilation::{Module, ModuleExternalImport};
use crate::compiler_frontend::utilities::basic::portable_path_text;
use std::collections::HashSet;

/// Metadata about one referenced external module export.
pub(super) struct ReferencedExport {
    pub(super) function_id: ExternalFunctionId,
    pub(super) package_id: ExternalPackageId,
    pub(super) export_name: String,
    pub(super) raw_import_name: String,
    pub(super) parameter_types: Vec<ExternalSignatureType>,
    pub(super) return_types: Vec<ExternalSignatureType>,
    pub(super) is_fallible: bool,
}

/// Checks selected JS inputs and collects only exports needing module wrappers.
pub(super) fn collect_referenced_exports(
    module: &Module,
    external_imports: &[ModuleExternalImport],
    referenced_external_functions: &HashSet<ExternalFunctionId>,
    registry: &ExternalPackageRegistry,
) -> Result<Vec<ReferencedExport>, CompilerError> {
    let mut exports = Vec::new();
    // Validate in emission order so corrupt inputs also produce deterministic failures.
    let mut ordered_function_ids: Vec<_> = referenced_external_functions.iter().collect();
    ordered_function_ids
        .sort_by_cached_key(|function_id| external_function_sort_key(**function_id));

    for function_id in ordered_function_ids {
        let function_def = registry.get_function_by_id(*function_id).ok_or_else(|| {
            selected_export_error(
                module,
                registry,
                *function_id,
                "referenced external function has no definition in the supplied registry",
            )
        })?;
        let lowering = function_def.lowerings.js.as_ref().ok_or_else(|| {
            selected_export_error(
                module,
                registry,
                *function_id,
                "referenced external function has no applicable JavaScript lowering",
            )
        })?;
        let ExternalJsLowering::ExternalModuleExport { export_name } = lowering else {
            continue;
        };
        let package_id = registry
            .resolve_function_package_id(*function_id)
            .ok_or_else(|| {
                selected_export_error(
                    module,
                    registry,
                    *function_id,
                    "selected external module export has no owning package identity",
                )
            })?;
        registry.get_package_by_id(package_id).ok_or_else(|| {
            selected_export_error(
                module,
                registry,
                *function_id,
                "selected external module export has no package definition",
            )
        })?;

        // Canonical collection owns metadata compatibility. Selection makes the JS asset
        // mandatory here, while inline and runtime helpers remain asset-free.
        let candidate = external_imports
            .iter()
            .find(|candidate| candidate.package_id == package_id)
            .ok_or_else(|| {
                selected_export_error(
                    module,
                    registry,
                    *function_id,
                    "selected external module export has no matching entry runtime-import candidate",
                )
            })?;
        let asset = candidate.runtime_asset.as_ref().ok_or_else(|| {
            selected_export_error(
                module,
                registry,
                *function_id,
                "matching entry runtime-import candidate has no runtime asset",
            )
        })?;
        if asset.asset_kind != "js" {
            return Err(selected_export_error(
                module,
                registry,
                *function_id,
                &format!(
                    "matching entry runtime-import candidate has asset kind '{}', expected 'js'",
                    asset.asset_kind
                ),
            ));
        }

        exports.push(ReferencedExport {
            function_id: *function_id,
            package_id,
            export_name: export_name.clone(),
            raw_import_name: raw_export_import_name(*function_id),
            parameter_types: function_def
                .parameters
                .iter()
                .map(|parameter| parameter.language_type.clone())
                .collect(),
            return_types: function_def
                .returns
                .iter()
                .map(|slot| slot.value_type.clone())
                .collect(),
            is_fallible: function_def.is_fallible(),
        });
    }

    Ok(exports)
}

/// Render bounded detail while the selected IDs are still paired with their registry.
fn selected_export_error(
    module: &Module,
    registry: &ExternalPackageRegistry,
    function_id: ExternalFunctionId,
    reason: &str,
) -> CompilerError {
    let function_name = registry
        .get_function_by_id(function_id)
        .map(|function| function.name.as_str())
        .unwrap_or("<unknown>");
    let export_name = match registry
        .get_function_by_id(function_id)
        .and_then(|function| function.lowerings.js.as_ref())
    {
        Some(ExternalJsLowering::ExternalModuleExport { export_name }) => export_name.as_str(),
        _ => "<unavailable>",
    };
    let package_path = registry
        .resolve_function_package(function_id)
        .unwrap_or("<unknown>");
    let package_id = registry.resolve_function_package_id(function_id);

    CompilerError::compiler_error(format!(
        "HTML JS glue preparation: validated runtime metadata is inconsistent for entry module '{}': \
         Moth function '{function_name}' ({function_id:?}), JS export '{export_name}', \
         package '{package_path}' ({package_id:?}): {reason}.",
        portable_path_text(&module.metadata.entry_point)
    ))
}

/// Produces a stable local import alias for a raw external export.
pub(super) fn raw_export_import_name(id: ExternalFunctionId) -> String {
    match id {
        ExternalFunctionId::Synthetic(n) => format!("__moth_external_fn{n}"),
        other => format!("__moth_external_{}", other.name()),
    }
}

/// Deterministic sort key for stable glue output ordering.
pub(super) fn external_function_sort_key(id: ExternalFunctionId) -> String {
    match id {
        ExternalFunctionId::Synthetic(n) => format!("synthetic:{n:010}"),
        other => format!("builtin:{}", other.name()),
    }
}
