//! Export-wrapper synthesis for Wasm lowering.

use crate::backends::error_types::lir_transformation_error;
use crate::backends::wasm::hir_to_lir::context::WasmLirLoweringContext;
use crate::backends::wasm::lir::function::{WasmLirBlock, WasmLirFunction, WasmLirFunctionOrigin};
use crate::backends::wasm::lir::instructions::{WasmCalleeRef, WasmLirStmt, WasmLirTerminator};
use crate::backends::wasm::lir::linkage::{WasmExport, WasmExportKind, WasmFunctionLinkage};
use crate::backends::wasm::lir::types::{
    WasmAbiType, WasmLirBlockId, WasmLirFunctionId, WasmLirLocal, WasmLirLocalId, WasmLocalRole,
};
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
use rustc_hash::FxHashSet;

pub(crate) fn synthesize_export_wrappers(
    context: &mut WasmLirLoweringContext<'_>,
) -> Result<(), CompilerError> {
    // WHAT: create synthetic wrapper functions for requested exports only.
    // WHY: keeps internal function linkage separate from externally stable names.
    // Wrappers round externally supplied F16 carriers before the internal call. Internal results
    // remain canonical because the callee produces them through the same scalar boundaries.
    if context.request.export_policy.exported_functions.is_empty() {
        return Ok(());
    }

    let mut seen_export_names = FxHashSet::default();

    for (wrapper_id, function_id) in (context.lir_module.functions.len() as u32..)
        .zip(context.request.export_policy.exported_functions.iter())
    {
        let export_name = context
            .request
            .export_policy
            .export_names
            .get(function_id)
            .ok_or_else(|| {
                lir_transformation_error(format!(
                    "Wasm export wrapper synthesis missing export name for {function_id:?}"
                ))
            })?;

        if !seen_export_names.insert(export_name.clone()) {
            return Err(lir_transformation_error(format!(
                "Wasm export wrapper synthesis encountered duplicate export name '{export_name}'"
            )));
        }

        let target_lir_id = context
            .function_map
            .get(function_id)
            .copied()
            .ok_or_else(|| {
                lir_transformation_error(format!(
                    "Wasm export wrapper synthesis could not resolve target function {function_id:?}"
                ))
            })?;

        // Target function must already be lowered before wrapper synthesis.
        let target_signature = context
            .lir_module
            .functions
            .iter()
            .find(|function| function.id == target_lir_id)
            .map(|function| function.signature.clone())
            .ok_or_else(|| {
                lir_transformation_error(format!(
                    "Wasm export wrapper synthesis missing lowered target function {target_lir_id:?}"
                ))
            })?;

        if target_signature.results.len() > 1 {
            return Err(lir_transformation_error(format!(
                "Wasm export wrapper synthesis does not yet support multi-value returns for {target_lir_id:?}"
            )));
        }
        let target_hir_function = context
            .hir_module
            .functions
            .iter()
            .find(|function| function.id == *function_id)
            .ok_or_else(|| {
                lir_transformation_error(format!(
                    "Wasm export wrapper synthesis could not resolve HIR function {function_id:?}"
                ))
            })?;
        let target_entry_block = context
            .hir_module
            .blocks
            .iter()
            .find(|block| block.id == target_hir_function.entry)
            .ok_or_else(|| {
                lir_transformation_error(format!(
                    "Wasm export wrapper synthesis could not resolve entry block for {function_id:?}"
                ))
            })?;
        if target_hir_function.params.len() != target_signature.params.len() {
            return Err(lir_transformation_error(format!(
                "Wasm export wrapper synthesis found inconsistent parameter counts for {function_id:?}"
            )));
        }

        // Allocate host parameters before canonicalization temporaries.
        // This keeps Wasm parameter indices aligned with the external signature.
        let mut locals = Vec::new();
        let mut args = Vec::new();
        let mut next_local_id = 0u32;

        for (index, abi) in target_signature.params.iter().enumerate() {
            let local_id = WasmLirLocalId(next_local_id);
            next_local_id += 1;
            locals.push(WasmLirLocal {
                id: local_id,
                name: Some(format!("arg{index}")),
                ty: *abi,
                role: WasmLocalRole::Param,
            });
            args.push(local_id);
        }

        let mut statements = Vec::new();
        for (index, parameter_local) in target_hir_function.params.iter().enumerate() {
            let parameter_type = target_entry_block
                .locals
                .iter()
                .find(|local| local.id == *parameter_local)
                .map(|local| local.ty)
                .ok_or_else(|| {
                    lir_transformation_error(format!(
                        "Wasm export wrapper synthesis could not resolve parameter local {parameter_local:?} for {function_id:?}"
                    ))
                })?;
            if context.type_environment.fixed_scalar(parameter_type) != Some(FixedScalar::F16) {
                continue;
            }
            if target_signature.params[index] != WasmAbiType::F32 {
                return Err(lir_transformation_error(format!(
                    "Wasm export wrapper synthesis found a non-F32 carrier for F16 parameter {parameter_local:?} in {function_id:?}"
                )));
            }

            let canonical_value = WasmLirLocalId(next_local_id);
            next_local_id += 1;
            locals.push(WasmLirLocal {
                id: canonical_value,
                name: None,
                ty: WasmAbiType::F32,
                role: WasmLocalRole::Temp,
            });
            statements.push(WasmLirStmt::RoundF16 {
                dst: canonical_value,
                source: args[index],
            });
            args[index] = canonical_value;
        }

        let result_local = target_signature.results.first().copied().map(|result_abi| {
            let local_id = WasmLirLocalId(next_local_id);
            locals.push(WasmLirLocal {
                id: local_id,
                name: Some("result".to_owned()),
                ty: result_abi,
                role: WasmLocalRole::Temp,
            });
            local_id
        });

        statements.push(WasmLirStmt::Call {
            dst: result_local,
            callee: WasmCalleeRef::Function(target_lir_id),
            args,
        });

        let wrapper_function_id = WasmLirFunctionId(wrapper_id);

        context.lir_module.functions.push(WasmLirFunction {
            id: wrapper_function_id,
            debug_name: format!("export_wrapper::{export_name}"),
            origin: WasmLirFunctionOrigin::ExportWrapper,
            signature: target_signature,
            locals,
            blocks: vec![WasmLirBlock {
                id: WasmLirBlockId(0),
                statements,
                terminator: WasmLirTerminator::Return {
                    value: result_local,
                },
            }],
            linkage: WasmFunctionLinkage::ExportedWrapper,
        });

        context.lir_module.exports.push(WasmExport {
            export_name: export_name.to_owned(),
            kind: WasmExportKind::Function(wrapper_function_id),
        });
    }

    Ok(())
}
