//! User-function code section emission.

use crate::backends::wasm::emit::helpers::emit_helper_function;
use crate::backends::wasm::emit::instructions::{
    LirBodyEmitContext, emit_statement, emit_terminator,
};
use crate::backends::wasm::emit::sections::{DefinedFunctionKey, WasmEmitPlan};
use crate::backends::wasm::emit::types::abi_to_val_type;
use crate::backends::wasm::lir::function::WasmLirFunction;
use crate::backends::wasm::lir::instructions::WasmLirTerminator;
use crate::backends::wasm::lir::types::{
    WasmAbiType, WasmLirBlockId, WasmLirLocalId, WasmLocalRole,
};
use crate::compiler_frontend::compiler_messages::compiler_errors::InfrastructureFailure;
use rustc_hash::FxHashMap;
use wasm_encoder::{CodeSection, Function, Instruction, ValType};

pub(crate) fn build_code_section(
    lir_functions: &FxHashMap<u32, &WasmLirFunction>,
    plan: &WasmEmitPlan,
) -> Result<CodeSection, InfrastructureFailure> {
    let mut section = CodeSection::new();

    // WHAT: iterate using the preplanned defined-function order.
    // WHY: function and code sections must stay index-aligned in Wasm binary encoding.
    for key in &plan.defined_function_order {
        let body = match key {
            DefinedFunctionKey::Lir(function_id) => {
                let function = lir_functions.get(&function_id.0).copied().ok_or_else(|| {
                    InfrastructureFailure::wasm_generation(format!(
                        "Wasm emission missing lowered function body for {function_id:?}"
                    ))
                })?;
                emit_lir_function(function, plan)?
            }
            DefinedFunctionKey::Helper(helper) => emit_helper_function(*helper, plan)?,
        };

        section.function(&body);
    }

    Ok(section)
}

fn emit_lir_function(
    function: &WasmLirFunction,
    plan: &WasmEmitPlan,
) -> Result<Function, InfrastructureFailure> {
    // WHAT: build deterministic local declarations and local-id -> local-index maps.
    // WHY: statement/terminator lowering depends on stable local index lookup.
    let local_layout = build_local_layout(function)?;
    let mut wasm_function = Function::new(local_layout.local_decls);

    let mut ordered_blocks = function.blocks.iter().collect::<Vec<_>>();
    ordered_blocks.sort_by_key(|block| block.id.0);

    let mut block_index_by_id = FxHashMap::default();
    for (index, block) in ordered_blocks.iter().enumerate() {
        block_index_by_id.insert(block.id, index as u32);
    }

    let entry_block = determine_entry_block(function)?;
    let entry_block_index = block_index_by_id
        .get(&entry_block)
        .copied()
        .ok_or_else(|| {
            InfrastructureFailure::wasm_generation(format!(
                "Wasm emission could not resolve entry block index for {:?} in {:?}",
                entry_block, function.id
            ))
        })?;

    wasm_function.instruction(&Instruction::I32Const(entry_block_index as i32));
    wasm_function.instruction(&Instruction::LocalSet(local_layout.dispatch_local_index));
    wasm_function.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
    wasm_function.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
    // WHY: the dispatcher loop supports arbitrary lowered CFG while leaving a seam for a
    // direct structured pass (`if/else/loop` region construction).

    let context = LirBodyEmitContext {
        function_id: function.id,
        local_index_by_id: &local_layout.local_index_by_id,
        local_type_by_id: &local_layout.local_type_by_id,
        block_index_by_id: &block_index_by_id,
        dispatch_local_index: local_layout.dispatch_local_index,
    };

    for (index, block) in ordered_blocks.iter().enumerate() {
        // WHAT: execute only the currently selected block in each loop iteration.
        // WHY: the dispatch local acts as a program counter for structured Wasm control flow.
        wasm_function.instruction(&Instruction::LocalGet(local_layout.dispatch_local_index));
        wasm_function.instruction(&Instruction::I32Const(index as i32));
        wasm_function.instruction(&Instruction::I32Eq);
        wasm_function.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));

        for statement in &block.statements {
            emit_statement(&mut wasm_function, statement, &context, plan)?;
        }
        emit_terminator(&mut wasm_function, &block.terminator, &context)?;

        wasm_function.instruction(&Instruction::End);
    }

    // Keeps dispatcher alive for any invalid/unknown dispatch value without falling out
    // of the loop and accidentally requiring an implicit function-end value.
    wasm_function.instruction(&Instruction::Br(0));
    wasm_function.instruction(&Instruction::End);
    wasm_function.instruction(&Instruction::End);
    wasm_function.instruction(&Instruction::Unreachable);
    wasm_function.instruction(&Instruction::End);

    Ok(wasm_function)
}

/// Deterministic local-index mapping and non-parameter declarations for a single function body.
struct LocalLayout {
    local_decls: Vec<(u32, ValType)>,
    local_index_by_id: FxHashMap<WasmLirLocalId, u32>,
    local_type_by_id: FxHashMap<WasmLirLocalId, WasmAbiType>,
    dispatch_local_index: u32,
}

fn build_local_layout(function: &WasmLirFunction) -> Result<LocalLayout, InfrastructureFailure> {
    // WHAT: local ids are sorted and mapped once for the entire function.
    // WHY: all statement lowering can then use fast, deterministic lookups.
    let mut sorted_locals = function.locals.iter().collect::<Vec<_>>();
    sorted_locals.sort_by_key(|local| local.id.0);

    let mut local_index_by_id = FxHashMap::default();
    let mut local_type_by_id = FxHashMap::default();
    for (index, local) in sorted_locals.iter().enumerate() {
        local_index_by_id.insert(local.id, index as u32);
        local_type_by_id.insert(local.id, local.ty);
    }

    let parameter_count = function.signature.params.len();
    if parameter_count > sorted_locals.len() {
        return Err(InfrastructureFailure::wasm_generation(format!(
            "Wasm emission found {} params in signature but only {} declared locals in {:?}",
            parameter_count,
            sorted_locals.len(),
            function.id
        )));
    }

    // WHAT: enforce Wasm local index contract: params first, then non-params.
    // WHY: this keeps local index mapping explicit and avoids accidental ABI drift.
    for (index, local) in sorted_locals.iter().enumerate() {
        if index < parameter_count && local.role != WasmLocalRole::Param {
            return Err(InfrastructureFailure::wasm_generation(format!(
                "Wasm emission expected local {:?} to be a parameter in {:?}",
                local.id, function.id
            )));
        }
        if index >= parameter_count && local.role == WasmLocalRole::Param {
            return Err(InfrastructureFailure::wasm_generation(format!(
                "Wasm emission found parameter local {:?} after non-parameter locals in {:?}",
                local.id, function.id
            )));
        }
    }

    let mut local_decls = Vec::new();
    let mut current_type = None;
    let mut current_count = 0u32;

    for local in sorted_locals.iter().skip(parameter_count) {
        let val_type = match local.ty {
            WasmAbiType::Void => ValType::I32,
            _ => abi_to_val_type(local.ty)?,
        };

        if current_type == Some(val_type) {
            current_count += 1;
        } else {
            if let Some(previous_type) = current_type {
                local_decls.push((current_count, previous_type));
            }
            current_type = Some(val_type);
            current_count = 1;
        }
    }

    if let Some(previous_type) = current_type {
        local_decls.push((current_count, previous_type));
    }

    // Dispatcher-local used by dispatcher-loop CFG lowering.
    local_decls.push((1, ValType::I32));
    let dispatch_local_index = sorted_locals.len() as u32;

    Ok(LocalLayout {
        local_decls,
        local_index_by_id,
        local_type_by_id,
        dispatch_local_index,
    })
}

fn determine_entry_block(
    function: &WasmLirFunction,
) -> Result<WasmLirBlockId, InfrastructureFailure> {
    // WHAT: prefer a unique zero-incoming root block as the entry block.
    // WHY: this reflects canonical CFG roots when lowering produced one clear entry.
    if function.blocks.is_empty() {
        return Err(InfrastructureFailure::wasm_generation(format!(
            "Wasm emission requires at least one block for function {:?}",
            function.id
        )));
    }

    let mut incoming_counts = FxHashMap::default();
    for block in &function.blocks {
        incoming_counts.entry(block.id).or_insert(0u32);
    }

    for block in &function.blocks {
        match block.terminator {
            WasmLirTerminator::Jump(target) => {
                *incoming_counts.entry(target).or_insert(0) += 1;
            }
            WasmLirTerminator::Branch {
                then_block,
                else_block,
                ..
            } => {
                *incoming_counts.entry(then_block).or_insert(0) += 1;
                *incoming_counts.entry(else_block).or_insert(0) += 1;
            }
            WasmLirTerminator::Return { .. } | WasmLirTerminator::Trap => {}
        }
    }

    let mut roots = incoming_counts
        .iter()
        .filter_map(|(block_id, incoming)| (*incoming == 0).then_some(*block_id))
        .collect::<Vec<_>>();
    roots.sort_by_key(|block_id| block_id.0);

    if roots.len() == 1 {
        return Ok(roots[0]);
    }

    if roots.is_empty() {
        // WHAT: fall back to the smallest block id in cyclic/fully-connected CFGs.
        // WHY: dispatcher-loop lowering can still execute correctly without a unique root.
        let mut sorted_blocks = function.blocks.iter().collect::<Vec<_>>();
        sorted_blocks.sort_by_key(|block| block.id.0);
        return Ok(sorted_blocks[0].id);
    }

    Err(InfrastructureFailure::wasm_generation(format!(
        "Wasm emission found multiple entry-block candidates in {:?}: {:?}",
        function.id, roots
    )))
}
