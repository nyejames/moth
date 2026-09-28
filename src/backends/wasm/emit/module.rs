//! Top-level LIR -> Wasm module emission orchestration.

use crate::backends::wasm::emit::data::build_data_section;
use crate::backends::wasm::emit::exports::build_export_section;
use crate::backends::wasm::emit::functions::build_code_section;
use crate::backends::wasm::emit::imports::build_import_section;
use crate::backends::wasm::emit::names::build_name_custom_section;
use crate::backends::wasm::emit::sections::{
    build_emit_plan, helper_exports_requested, plan_data_layout_text, plan_indices_text,
    plan_sections_text,
};
use crate::backends::wasm::emit::types::abi_to_val_type;
use crate::backends::wasm::emit::validate::validate_emitted_module;
use crate::backends::wasm::emit::{WasmEmitDebugOutputs, WasmEmitResult};
use crate::backends::wasm::lir::module::WasmLirModule;
use crate::backends::wasm::request::WasmBackendRequest;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use rustc_hash::FxHashMap;
use wasm_encoder::{
    ConstExpr, FunctionSection, GlobalSection, GlobalType, MemorySection, MemoryType, Module,
    TypeSection,
};

pub(crate) fn emit_lir_to_wasm_module(
    lir_module: &WasmLirModule,
    request: &WasmBackendRequest,
) -> Result<WasmEmitResult, CompilerError> {
    // WHAT: precompute all indices/layout before section writing starts.
    // WHY: index spaces cross-reference each other, so deterministic preplanning avoids
    // accidental order-dependent bugs during section assembly.
    let mut plan = build_emit_plan(lir_module, request)?;
    let mut wasm_module = Module::new();

    // WHAT: type section must be emitted first in core Wasm section order.
    // WHY: function/import sections reference these type indices.
    let mut type_section = TypeSection::new();
    for signature in &plan.type_entries {
        let mut params = Vec::with_capacity(signature.params.len());
        for param in &signature.params {
            params.push(abi_to_val_type(*param)?);
        }

        let mut results = Vec::with_capacity(signature.results.len());
        for result in &signature.results {
            results.push(abi_to_val_type(*result)?);
        }

        type_section.ty().function(params, results);
    }
    if !plan.type_entries.is_empty() {
        wasm_module.section(&type_section);
    }

    // WHAT: emit imports only when present in the lowered module.
    // WHY: the current emitter keeps binary output minimal and deterministic.
    if !lir_module.imports.is_empty() {
        let import_section = build_import_section(lir_module, &plan)?;
        wasm_module.section(&import_section);
    }

    // WHAT: function section lists type indices for each defined function body.
    // WHY: imported functions already occupy leading function indices.
    if !plan.defined_function_order.is_empty() {
        let mut function_section = FunctionSection::new();
        for type_index in &plan.defined_function_type_indices {
            function_section.function(*type_index);
        }
        wasm_module.section(&function_section);
    }

    let mut memory_section = MemorySection::new();
    memory_section.memory(MemoryType {
        minimum: u64::from(plan.initial_memory_pages),
        maximum: lir_module.memory_plan.max_pages.map(u64::from),
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    wasm_module.section(&memory_section);

    // WHAT: heap_top global is synthesized only when helper runtime support is needed.
    // WHY: helpers use bump allocation, while modules with no helper usage can skip globals.
    if plan.heap_top_global_index.is_some() {
        let mut global_section = GlobalSection::new();
        global_section.global(
            GlobalType {
                val_type: wasm_encoder::ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(plan.heap_base as i32),
        );
        wasm_module.section(&global_section);
    }

    let has_exports = !lir_module.exports.is_empty() || helper_exports_requested(request);
    if has_exports {
        let export_section = build_export_section(lir_module, &plan, request)?;
        wasm_module.section(&export_section);
    }

    let mut lir_functions = FxHashMap::default();
    for function in &lir_module.functions {
        lir_functions.insert(function.id.0, function);
    }
    // WHAT: code section mirrors `defined_function_order` exactly.
    // WHY: function section and code section must stay index-aligned.
    if !plan.defined_function_order.is_empty() {
        let code_section = build_code_section(&lir_functions, &plan)?;
        wasm_module.section(&code_section);
    }

    // WHAT: static data is emitted as active segments into memory index 0.
    // WHY: the current emitter uses one internal linear memory and deterministic static placement.
    if !lir_module.static_data.is_empty() || plan.float_format_tables_offset.is_some() {
        let data_section = build_data_section(lir_module, &mut plan)?;
        wasm_module.section(&data_section);
    }

    // WHAT: name custom section is optional and debug-oriented only.
    // WHY: binaries remain minimal by default while keeping an opt-in diagnostics hook.
    if request.emit_options.emit_name_section {
        let name_section = build_name_custom_section();
        wasm_module.section(&name_section);
    }

    // WHAT: we intentionally do not emit a start section.
    // WHY: builder/host orchestration should explicitly control entry invocation.
    let wasm_bytes = wasm_module.finish();
    let validation_text = if request.emit_options.validate_emitted_module {
        Some(validate_emitted_module(&wasm_bytes)?)
    } else {
        None
    };

    let mut sorted_lir_functions = lir_module.functions.iter().collect::<Vec<_>>();
    sorted_lir_functions.sort_by_key(|function| function.id.0);

    Ok(WasmEmitResult {
        wasm_bytes,
        debug_outputs: WasmEmitDebugOutputs {
            sections_text: plan_sections_text(lir_module, &plan),
            indices_text: plan_indices_text(lir_module, &plan, &sorted_lir_functions),
            data_layout_text: plan_data_layout_text(lir_module, &plan),
            validation_text,
        },
    })
}
