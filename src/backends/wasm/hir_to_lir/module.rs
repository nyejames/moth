//! Module-level orchestration for HIR -> LIR lowering.

use crate::backends::wasm::hir_to_lir::context::WasmLirLoweringContext;
use crate::backends::wasm::hir_to_lir::exports::synthesize_export_wrappers;
use crate::backends::wasm::hir_to_lir::function::lower_function;
use crate::backends::wasm::hir_to_lir::imports::register_required_host_imports;
use crate::backends::wasm::lir::types::WasmLirFunctionId;
use crate::backends::wasm::request::{WasmBackendRequest, WasmFunctionEmissionPolicy};
use crate::compiler_frontend::analysis::borrow_checker::BorrowFacts;
use crate::compiler_frontend::compiler_messages::compiler_errors::{
    CompilerError, CompilerMessages,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::BlockId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::FxHashSet;

pub(crate) fn lower_hir_module_to_lir(
    hir_module: &HirModule,
    borrow_facts: &BorrowFacts,
    request: &WasmBackendRequest,
    string_table: &StringTable,
    type_environment: &TypeEnvironment,
) -> Result<crate::backends::wasm::lir::module::WasmLirModule, CompilerMessages> {
    // WHAT: one mutable context carries all per-module lowering state.
    // WHY: keeps interning/ids/import planning coherent and deterministic.
    let mut context = WasmLirLoweringContext::new(
        hir_module,
        borrow_facts,
        request,
        string_table,
        type_environment,
    );
    let function_selection = select_functions_for_lowering(hir_module, request);

    // WHAT: register stable function mappings before lowering any bodies.
    // WHY: call lowering depends on these mappings even for forward references.
    register_function_maps(&mut context, &function_selection.functions)
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;
    // WHAT: pre-register host imports required by the module.
    // WHY: keeps import ids deterministic and avoids re-scan during statement lowering.
    register_required_host_imports(&mut context, function_selection.reachable_blocks.as_ref())
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;

    for hir_function in &function_selection.functions {
        let selected_blocks = match &request.function_emission_policy {
            WasmFunctionEmissionPolicy::AllFunctions => None,
            WasmFunctionEmissionPolicy::Selected(selection) => {
                selection.blocks_for_function(hir_function.id)
            }
        };
        let lowered = lower_function(&mut context, hir_function, selected_blocks)
            .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;
        context.lir_module.functions.push(lowered);
    }

    // WHAT: synthesize post-lowering export wrapper functions.
    // WHY: wrapper boundary keeps user bodies internal while export ABI stays stable.
    synthesize_export_wrappers(&mut context)
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;

    Ok(context.lir_module)
}

struct WasmFunctionLoweringSelection {
    functions: Vec<HirFunction>,
    reachable_blocks: Option<FxHashSet<BlockId>>,
}

fn select_functions_for_lowering(
    hir_module: &HirModule,
    request: &WasmBackendRequest,
) -> WasmFunctionLoweringSelection {
    // WHAT: choose the backend function set before assigning LIR ids.
    // WHY: HTML-Wasm page bundles must not lower unused source-backed package wrappers, while the
    // generic Wasm backend keeps its existing all-functions default for direct tests.
    match &request.function_emission_policy {
        WasmFunctionEmissionPolicy::AllFunctions => WasmFunctionLoweringSelection {
            functions: sorted_functions(hir_module.functions.clone()),
            reachable_blocks: None,
        },

        WasmFunctionEmissionPolicy::Selected(selection) => {
            let mut functions = hir_module.functions.clone();
            functions.retain(|function| selection.contains_function(function.id));

            WasmFunctionLoweringSelection {
                functions: sorted_functions(functions),
                reachable_blocks: Some(selection.blocks().clone()),
            }
        }
    }
}

fn sorted_functions(mut functions: Vec<HirFunction>) -> Vec<HirFunction> {
    functions.sort_by_key(|function| function.id.0);
    functions
}

fn register_function_maps(
    context: &mut WasmLirLoweringContext<'_>,
    functions: &[HirFunction],
) -> Result<(), CompilerError> {
    // This mapping is intentionally explicit so lowering, code-section ordering, and debug
    // correlation all share the same stable function ids.
    for (next_id, function) in functions.iter().enumerate() {
        context
            .function_map
            .insert(function.id, WasmLirFunctionId(next_id as u32));
    }

    Ok(())
}
