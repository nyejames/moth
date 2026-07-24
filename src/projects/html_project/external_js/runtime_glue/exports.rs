//! Referenced export collection for HTML JS glue.
//!
//! WHAT: identifies which externally-referenced functions use `ExternalModuleExport` lowering
//!       and produces deterministic, sorted metadata for glue generation.
//! WHY: the JS backend may reference many external functions, but only module exports need
//!      generated glue wrappers.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::{
    ExternalFunctionId, ExternalJsLowering, ExternalPackageId, ExternalPackageRegistry,
};
use std::collections::HashSet;

/// Metadata about one referenced external module export.
pub(super) struct ReferencedExport {
    pub(super) function_id: ExternalFunctionId,
    pub(super) package_id: ExternalPackageId,
    pub(super) export_name: String,
    pub(super) raw_import_name: String,
    pub(super) is_fallible: bool,
}

/// Collects the subset of referenced external functions that use `ExternalModuleExport`.
pub(super) fn collect_referenced_exports(
    referenced_external_functions: &HashSet<ExternalFunctionId>,
    registry: &ExternalPackageRegistry,
) -> Result<Vec<ReferencedExport>, CompilerError> {
    let mut exports = Vec::new();

    for function_id in referenced_external_functions {
        let Some(function_def) = registry.get_function_by_id(*function_id) else {
            continue;
        };
        let Some(lowering) = function_def.lowerings.js.as_ref() else {
            continue;
        };
        let ExternalJsLowering::ExternalModuleExport { export_name } = lowering else {
            continue;
        };
        let package_id = registry
            .resolve_function_package_id(*function_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "HTML JS glue could not resolve a package for external function '{}'.",
                    function_id.name()
                ))
            })?;

        exports.push(ReferencedExport {
            function_id: *function_id,
            package_id,
            export_name: export_name.clone(),
            raw_import_name: raw_export_import_name(*function_id),
            is_fallible: function_def.is_fallible(),
        });
    }

    exports.sort_by(|left, right| {
        external_function_sort_key(left.function_id)
            .cmp(&external_function_sort_key(right.function_id))
    });

    Ok(exports)
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
