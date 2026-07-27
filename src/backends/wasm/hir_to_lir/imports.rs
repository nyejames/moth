//! Host import planning for HIR -> LIR lowering.

use crate::backends::error_types::lir_transformation_error;
use crate::backends::wasm::hir_to_lir::context::WasmLirLoweringContext;
use crate::backends::wasm::lir::linkage::{WasmImport, WasmImportKind};
use crate::backends::wasm::lir::types::{WasmImportId, WasmLirSignature};
use crate::backends::wasm::runtime::imports::WasmHostFunction;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalFunctionId};
use crate::compiler_frontend::hir::ids::BlockId;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use rustc_hash::FxHashSet;

pub(crate) fn register_required_host_imports(
    context: &mut WasmLirLoweringContext<'_>,
    reachable_blocks: Option<&FxHashSet<BlockId>>,
) -> Result<(), CompilerError> {
    // WHAT: scan HIR for host calls and pre-register required imports.
    // WHY: deterministic import-id assignment for the whole module.
    for block in &context.hir_module.blocks {
        if let Some(reachable_blocks) = reachable_blocks
            && !reachable_blocks.contains(&block.id)
        {
            continue;
        }

        for statement in &block.statements {
            if let HirStatementKind::Call {
                target: CallTarget::External(id),
                ..
            } = &statement.kind
            {
                let host_function = resolve_host_function_id(context, *id)?;
                ensure_host_import(context, host_function);
            }
        }
    }

    Ok(())
}

pub(crate) fn resolve_host_call_import(
    context: &mut WasmLirLoweringContext<'_>,
    target: &CallTarget,
) -> Result<WasmImportId, CompilerError> {
    // WHAT: resolve a host call target to its pre-registered import id.
    // WHY: each distinct host function maps to exactly one import; unsupported targets
    // must fail with a structured diagnostic instead of silently mapping to the wrong import.
    let CallTarget::External(id) = target else {
        return Err(lir_transformation_error(
            "Wasm lowering expected a HostFunction call target in resolve_host_call_import",
        ));
    };

    let host_function = resolve_host_function_id(context, *id)?;
    Ok(ensure_host_import(context, host_function))
}

fn resolve_host_function_id(
    context: &WasmLirLoweringContext<'_>,
    id: ExternalFunctionId,
) -> Result<WasmHostFunction, CompilerError> {
    // WHAT: map a host function id to its Wasm backend import identity.
    // WHY: ensures only explicitly supported host calls are lowered.
    let function_name = context
        .request
        .external_package_registry
        .get_function_by_id(id)
        .map(|function| function.name.clone())
        .unwrap_or_else(|| id.name().to_owned());
    Err(lir_transformation_error(format!(
        "Wasm backend does not yet support host function '{function_name}'"
    )))
}

fn ensure_host_import(
    context: &mut WasmLirLoweringContext<'_>,
    function: WasmHostFunction,
) -> WasmImportId {
    // Idempotent insert: same host function always maps to same import id.
    if let Some(import_id) = context.host_imports.get(&function).copied() {
        return import_id;
    }

    let import_id = WasmImportId(context.lir_module.imports.len() as u32);
    context.host_imports.insert(function, import_id);

    // WHAT: import signature is determined by the host function identity.
    // WHY: each host function has a fixed ABI contract.
    let signature = host_function_signature(function);
    context.lir_module.imports.push(WasmImport {
        id: import_id,
        module_name: function.module_name().to_owned(),
        item_name: function.item_name().to_owned(),
        kind: WasmImportKind::Function(signature),
    });

    import_id
}

fn host_function_signature(function: WasmHostFunction) -> WasmLirSignature {
    // WHAT: canonical ABI signature for each supported host function.
    // WHY: keeps import registration and signature assignment in one explicit place.
    match function {}
}
