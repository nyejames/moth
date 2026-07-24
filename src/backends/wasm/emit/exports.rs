//! Export section emission.

use crate::backends::error_types::BackendErrorType;
use crate::backends::wasm::emit::sections::WasmEmitPlan;
use crate::backends::wasm::lir::linkage::WasmExportKind;
use crate::backends::wasm::lir::module::WasmLirModule;
use crate::backends::wasm::request::WasmBackendRequest;
use crate::backends::wasm::runtime::strings::WasmRuntimeHelper;
use crate::compiler_frontend::compiler_messages::compiler_errors::{CompilerError, ErrorType};
use wasm_encoder::{ExportKind, ExportSection};

pub(crate) fn build_export_section(
    module: &WasmLirModule,
    plan: &WasmEmitPlan,
    request: &WasmBackendRequest,
) -> Result<ExportSection, CompilerError> {
    let mut section = ExportSection::new();

    // WHAT: emit LIR-declared exports first.
    // WHY: project/build orchestration owns frontend-facing API naming decisions.
    for export in &module.exports {
        match export.kind {
            WasmExportKind::Function(function_id) => {
                let function_index = plan
                    .function_indices
                    .get(&function_id)
                    .copied()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "Wasm emission could not resolve function index for export '{function_id:?}'"
                        ))
                        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
                    })?;
                section.export(
                    export.export_name.as_str(),
                    ExportKind::Func,
                    function_index,
                );
            }
        }
    }

    let helper_exports = &request.export_policy.helper_exports;
    // WHAT: helper export names are fixed for host/runtime compatibility.
    // WHY: HTML/Wasm integration depends on stable helper symbol names.
    if helper_exports.export_memory {
        section.export("memory", ExportKind::Memory, 0);
    }
    if helper_exports.export_str_ptr {
        section.export(
            "moth_str_ptr",
            ExportKind::Func,
            helper_index(plan, WasmRuntimeHelper::StringPtr)?,
        );
    }
    if helper_exports.export_str_len {
        section.export(
            "moth_str_len",
            ExportKind::Func,
            helper_index(plan, WasmRuntimeHelper::StringLen)?,
        );
    }
    if helper_exports.export_vec_new {
        section.export(
            "moth_vec_new",
            ExportKind::Func,
            helper_index(plan, WasmRuntimeHelper::VecNew)?,
        );
    }
    if helper_exports.export_vec_push {
        section.export(
            "moth_vec_push",
            ExportKind::Func,
            helper_index(plan, WasmRuntimeHelper::VecPushHandle)?,
        );
    }
    if helper_exports.export_vec_len {
        section.export(
            "moth_vec_len",
            ExportKind::Func,
            helper_index(plan, WasmRuntimeHelper::VecLen)?,
        );
    }
    if helper_exports.export_vec_get {
        section.export(
            "moth_vec_get",
            ExportKind::Func,
            helper_index(plan, WasmRuntimeHelper::VecGet)?,
        );
    }
    if helper_exports.export_release {
        section.export(
            "moth_release",
            ExportKind::Func,
            helper_index(plan, WasmRuntimeHelper::Release)?,
        );
    }

    Ok(section)
}

fn helper_index(plan: &WasmEmitPlan, helper: WasmRuntimeHelper) -> Result<u32, CompilerError> {
    plan.helper_indices.get(&helper).copied().ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "Wasm emission missing helper function index for {}",
            crate::backends::wasm::emit::sections::helper_name(helper)
        ))
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
    })
}
