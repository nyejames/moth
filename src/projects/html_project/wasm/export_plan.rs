//! Export-plan construction for HTML builder Wasm mode.
//!
//! WHAT: selects which functions must be callable from builder-owned JS orchestration.
//! WHY: the backend must stay generic and only lower exports explicitly requested by builders.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::module::HirModule;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HtmlWasmExportPlan {
    /// Deterministic function export assignments used by JS wrapper generation.
    pub function_exports: Vec<HtmlWasmFunctionExport>,
    /// Helper exports required for string interop and memory access from JS.
    pub helper_exports: HtmlWasmHelperExports,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HtmlWasmFunctionExport {
    /// Function selected from HIR as callable from builder-owned JS orchestration.
    pub function_id: FunctionId,
    /// Stable export symbol name exposed by Wasm.
    pub export_name: String,
    /// Reason the function is exported, used for debug readability and test intent.
    pub purpose: HtmlWasmExportPurpose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HtmlWasmExportPurpose {
    /// Entry start() exported directly; JS calls it and decodes the returned fragment list.
    EntryStart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HtmlWasmHelperExports {
    /// Export linear memory so JS can decode backend-managed UTF-8 buffers.
    pub export_memory: bool,
    /// Export helper that returns string buffer pointer for a string handle.
    pub export_str_ptr: bool,
    /// Export helper that returns string byte length for a string handle.
    pub export_str_len: bool,
    /// Export helper that allocates a runtime fragment Vec handle.
    pub export_vec_new: bool,
    /// Export helper that appends a string handle to a runtime fragment Vec handle.
    pub export_vec_push: bool,
    /// Export helper that returns the runtime fragment Vec length.
    pub export_vec_len: bool,
    /// Export helper that reads one runtime fragment string handle from the Vec.
    pub export_vec_get: bool,
    /// Export helper that releases a moved string handle after JS consumption.
    pub export_release: bool,
}

impl HtmlWasmHelperExports {
    /// Enables all currently required helpers for HTML Wasm mode.
    ///
    /// WHAT: turns on the full string interop helper surface.
    /// WHY: phase-1 HTML Wasm mode always depends on these helpers.
    pub(crate) fn all_enabled() -> Self {
        Self {
            export_memory: true,
            export_str_ptr: true,
            export_str_len: true,
            export_vec_new: true,
            export_vec_push: true,
            export_vec_len: true,
            export_vec_get: true,
            export_release: true,
        }
    }
}

/// Builds the full HTML->Wasm export plan from builder-visible HIR semantics.
///
/// WHAT: exports entry start() directly as "moth_start" plus all string interop helpers.
/// WHY: entry start() is the sole runtime fragment producer. JS calls it once and decodes
///      the returned fragment list. No entry-body call scanning is needed or correct.
pub(crate) fn build_html_wasm_export_plan(
    hir_module: &HirModule,
) -> Result<HtmlWasmExportPlan, CompilerError> {
    Ok(HtmlWasmExportPlan {
        function_exports: vec![HtmlWasmFunctionExport {
            function_id: hir_module.start_function,
            export_name: String::from("moth_start"),
            purpose: HtmlWasmExportPurpose::EntryStart,
        }],
        helper_exports: HtmlWasmHelperExports::all_enabled(),
    })
}
