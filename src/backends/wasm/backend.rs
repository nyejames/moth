//! Top-level orchestration for HIR -> LIR and optional LIR -> Wasm emission.

use crate::backends::error_types::{BackendErrorType, lir_transformation_error};
use crate::backends::wasm::debug::build_debug_outputs;
use crate::backends::wasm::emit::module::emit_lir_to_wasm_module;
use crate::backends::wasm::hir_to_lir::module::lower_hir_module_to_lir;
use crate::backends::wasm::request::{
    WasmBackendRequest, WasmCfgLoweringStrategy, WasmFunctionEmissionPolicy,
};
use crate::backends::wasm::result::WasmLirBackendResult;
use crate::compiler_frontend::analysis::borrow_checker::BorrowFacts;
use crate::compiler_frontend::compiler_messages::compiler_errors::{
    CompilerError, CompilerMessages, ErrorType,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::collections::HashSet;

pub(crate) fn lower_hir_to_wasm_lir(
    hir_module: &HirModule,
    borrow_facts: &BorrowFacts,
    request: &WasmBackendRequest,
    string_table: &StringTable,
    type_environment: &TypeEnvironment,
) -> Result<WasmLirBackendResult, CompilerMessages> {
    // WHAT: fail fast on builder/backend contract issues.
    // WHY: avoid partial lowering and keep diagnostics deterministic.
    validate_request(hir_module, request)
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;

    // WHAT: perform full module lowering using HIR + borrow side tables.
    // WHY: LIR is the stable inspection and emission seam for the experimental Wasm backend.
    let lir_module = lower_hir_module_to_lir(
        hir_module,
        borrow_facts,
        request,
        string_table,
        type_environment,
    )?;
    // WHAT: collect optional debug text with zero impact on lowering semantics.
    let debug_outputs = build_debug_outputs(request, &lir_module);

    Ok(WasmLirBackendResult {
        lir_module,
        wasm_bytes: None,
        debug_outputs,
    })
}

pub(crate) fn lower_hir_to_wasm_module(
    hir_module: &HirModule,
    borrow_facts: &BorrowFacts,
    request: &WasmBackendRequest,
    string_table: &StringTable,
    type_environment: &TypeEnvironment,
) -> Result<WasmLirBackendResult, CompilerMessages> {
    // WHAT: preserve the LIR lowering entry and layer byte emission on top.
    // WHY: this keeps debug and diagnostics workflows stable while emission matures.
    let mut result = lower_hir_to_wasm_lir(
        hir_module,
        borrow_facts,
        request,
        string_table,
        type_environment,
    )?;
    if !request.emit_options.emit_wasm_module {
        return Ok(result);
    }

    // WHAT: perform pure Wasm encoding from already-lowered LIR.
    // WHY: emitter must stay backend-encoding focused and avoid reinterpreting frontend semantics.
    let emit_result = emit_lir_to_wasm_module(&result.lir_module, request)
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;
    result.wasm_bytes = Some(emit_result.wasm_bytes);

    if request.debug_flags.show_wasm_sections {
        result.debug_outputs.wasm_sections_text = Some(emit_result.debug_outputs.sections_text);
    }
    if request.debug_flags.show_wasm_indices {
        result.debug_outputs.wasm_indices_text = Some(emit_result.debug_outputs.indices_text);
    }
    if request.debug_flags.show_wasm_data_layout {
        result.debug_outputs.wasm_data_layout_text =
            Some(emit_result.debug_outputs.data_layout_text);
    }
    if request.debug_flags.show_wasm_validation {
        result.debug_outputs.wasm_validation_text = emit_result.debug_outputs.validation_text;
    }

    Ok(result)
}

fn validate_request(
    hir_module: &HirModule,
    request: &WasmBackendRequest,
) -> Result<(), CompilerError> {
    // WHAT: reject request/contract issues before any lowering or emission work.
    // WHY: this guarantees deterministic diagnostics and prevents partial outputs.
    let mut seen = HashSet::new();
    let mut export_name_set = HashSet::new();

    if let WasmFunctionEmissionPolicy::Selected(selection) = &request.function_emission_policy {
        selection.validate_for_hir(hir_module)?;
    }

    for function_id in &request.export_policy.exported_functions {
        if !seen.insert(function_id.0) {
            return Err(lir_transformation_error(format!(
                "Wasm backend request contains duplicate export target {function_id:?}",
            )));
        }

        if !contains_function(hir_module, *function_id) {
            return Err(lir_transformation_error(format!(
                "Wasm backend request references missing function {function_id:?}",
            )));
        }

        if let WasmFunctionEmissionPolicy::Selected(selection) = &request.function_emission_policy
            && !selection.contains_function(*function_id)
        {
            return Err(lir_transformation_error(format!(
                "Wasm backend export {function_id:?} is absent from the selected function plan"
            )));
        }

        let Some(export_name) = request.export_policy.export_names.get(function_id) else {
            return Err(lir_transformation_error(format!(
                "Wasm backend request missing stable export name for {function_id:?}",
            )));
        };
        if !export_name_set.insert(export_name.to_owned()) {
            return Err(lir_transformation_error(format!(
                "Wasm backend request contains duplicate export name '{export_name}'",
            )));
        }
    }

    for (function_id, export_name) in &request.export_policy.export_names {
        if !contains_function(hir_module, *function_id) {
            return Err(lir_transformation_error(format!(
                "Wasm backend request has export name entry for unknown function {function_id:?}"
            )));
        }

        if export_name.trim().is_empty() {
            return Err(lir_transformation_error(format!(
                "Wasm backend request contains an empty export name for {function_id:?}"
            )));
        }

        if !seen.contains(&function_id.0) {
            return Err(lir_transformation_error(format!(
                "Wasm backend request has export name entry for {function_id:?} but it is not in exported_functions"
            )));
        }
    }

    validate_feature_flags(request)?;
    validate_emit_options(request)?;
    validate_helper_export_policy(&mut export_name_set, request)?;

    Ok(())
}

fn contains_function(hir_module: &HirModule, function_id: FunctionId) -> bool {
    hir_module
        .functions
        .iter()
        .any(|function| function.id == function_id)
}

fn validate_feature_flags(request: &WasmBackendRequest) -> Result<(), CompilerError> {
    // WHAT: reject feature toggles that would require a different emitter/runtime model.
    // WHY: rejecting incompatible toggles early keeps backend behavior explicit and predictable.
    if request.target_features.use_wasm_gc {
        return Err(CompilerError::compiler_error(
            "Wasm backend request enables use_wasm_gc, but the current emitter targets core linear-memory Wasm only",
        )
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
    }

    if request.target_features.enable_multi_value {
        return Err(CompilerError::compiler_error(
            "Wasm backend request enables multi-value, but the current ABI supports single-result only",
        )
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
    }

    if request.target_features.enable_reference_types {
        return Err(CompilerError::compiler_error(
            "Wasm backend request enables reference types, but the current emitter targets core value types only",
        )
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
    }

    // Note: bulk-memory (memory.copy) is now used internally by runtime string helpers.
    // The enable_bulk_memory flag is reserved for user-facing feature gating if needed.

    Ok(())
}

fn validate_emit_options(request: &WasmBackendRequest) -> Result<(), CompilerError> {
    if matches!(
        request.emit_options.cfg_lowering_strategy,
        WasmCfgLoweringStrategy::Structured
    ) {
        return Err(CompilerError::compiler_error(
            "Wasm backend request selected structured CFG lowering, but the current emitter implements dispatcher-loop CFG lowering only",
        )
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
    }

    Ok(())
}

fn validate_helper_export_policy(
    export_name_set: &mut HashSet<String>,
    request: &WasmBackendRequest,
) -> Result<(), CompilerError> {
    // WHAT: enforce helper-export contract invariants required by HTML/Wasm glue.
    // WHY: keeping names/combinations strict here avoids backend-side guessing later.
    let helpers = &request.export_policy.helper_exports;

    if helpers.export_str_ptr != helpers.export_str_len {
        return Err(CompilerError::compiler_error(
            "Wasm helper exports must request both moth_str_ptr and moth_str_len together",
        )
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
    }

    if helpers.export_vec_new != helpers.export_vec_push {
        return Err(CompilerError::compiler_error(
            "Wasm helper exports must request both moth_vec_new and moth_vec_push together",
        )
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
    }

    if helpers.export_vec_len != helpers.export_vec_get {
        return Err(CompilerError::compiler_error(
            "Wasm helper exports must request both moth_vec_len and moth_vec_get together",
        )
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
    }

    if (helpers.export_str_ptr || helpers.export_str_len) && !helpers.export_memory {
        return Err(CompilerError::compiler_error(
            "Wasm helper exports requesting string pointer/length must also export memory",
        )
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
    }

    let mut add_reserved_export =
        |enabled: bool, name: &'static str| -> Result<(), CompilerError> {
            if !enabled {
                return Ok(());
            }

            if !export_name_set.insert(name.to_owned()) {
                return Err(CompilerError::compiler_error(format!(
                    "Wasm helper export '{name}' collides with an existing function export name",
                ))
                .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
            }

            Ok(())
        };

    add_reserved_export(helpers.export_memory, "memory")?;
    add_reserved_export(helpers.export_str_ptr, "moth_str_ptr")?;
    add_reserved_export(helpers.export_str_len, "moth_str_len")?;
    add_reserved_export(helpers.export_vec_new, "moth_vec_new")?;
    add_reserved_export(helpers.export_vec_push, "moth_vec_push")?;
    add_reserved_export(helpers.export_vec_len, "moth_vec_len")?;
    add_reserved_export(helpers.export_vec_get, "moth_vec_get")?;
    add_reserved_export(helpers.export_release, "moth_release")?;

    Ok(())
}
