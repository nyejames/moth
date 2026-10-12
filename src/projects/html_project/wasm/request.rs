//! Request-construction helpers for the generic Wasm backend contract.
//!
//! WHAT: converts builder-owned export planning into `WasmBackendRequest`.
//! WHY: keeps backend contract translation in one place as request fields evolve.

use crate::backends::wasm::request::{
    WasmBackendRequest, WasmExportPolicy, WasmFunctionEmissionPolicy, WasmHelperExportPolicy,
};
use crate::compiler_frontend::hir::reachability::HirReachability;
use crate::projects::html_project::wasm::export_plan::HtmlWasmExportPlan;
use rustc_hash::FxHashMap;

/// Builds the generic backend request from HTML builder export planning.
///
/// WHAT: copies deterministic export IDs/names plus required helper exports.
/// WHY: request-building stays centralized so builder policy is translated once.
pub(crate) fn build_wasm_backend_request(
    export_plan: &HtmlWasmExportPlan,
    reachability: &HirReachability,
) -> WasmBackendRequest {
    let mut export_names = FxHashMap::default();
    let mut exported_functions = Vec::with_capacity(export_plan.function_exports.len());

    for function_export in &export_plan.function_exports {
        exported_functions.push(function_export.function_id);
        export_names.insert(
            function_export.function_id,
            function_export.export_name.clone(),
        );
    }

    WasmBackendRequest {
        export_policy: WasmExportPolicy {
            exported_functions,
            export_names,
            helper_exports: WasmHelperExportPolicy {
                export_memory: export_plan.helper_exports.export_memory,
                export_str_ptr: export_plan.helper_exports.export_str_ptr,
                export_str_len: export_plan.helper_exports.export_str_len,
                export_vec_new: export_plan.helper_exports.export_vec_new,
                export_vec_push: export_plan.helper_exports.export_vec_push,
                export_vec_len: export_plan.helper_exports.export_vec_len,
                export_vec_get: export_plan.helper_exports.export_vec_get,
                export_release: export_plan.helper_exports.export_release,
            },
        },
        function_emission_policy: WasmFunctionEmissionPolicy::Selected(
            reachability.backend_selection().clone(),
        ),
        ..WasmBackendRequest::default()
    }
}
