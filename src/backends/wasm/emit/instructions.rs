//! Instruction lowering from Wasm LIR to wasm-encoder instructions.

use crate::backends::error_types::BackendErrorType;
use crate::backends::wasm::emit::sections::WasmEmitPlan;
use crate::backends::wasm::lir::instructions::{WasmCalleeRef, WasmLirStmt, WasmLirTerminator};
use crate::backends::wasm::lir::types::{
    WasmAbiType, WasmLirBlockId, WasmLirFunctionId, WasmLirLocalId,
};
use crate::backends::wasm::runtime::strings::WasmRuntimeHelper;
use crate::compiler_frontend::compiler_messages::compiler_errors::{CompilerError, ErrorType};
use rustc_hash::FxHashMap;
use wasm_encoder::{BlockType, Function, Instruction, ValType};

pub(crate) struct LirBodyEmitContext<'a> {
    pub function_id: WasmLirFunctionId,
    pub local_index_by_id: &'a FxHashMap<WasmLirLocalId, u32>,
    pub local_type_by_id: &'a FxHashMap<WasmLirLocalId, WasmAbiType>,
    pub block_index_by_id: &'a FxHashMap<WasmLirBlockId, u32>,
    pub dispatch_local_index: u32,
}

pub(crate) fn emit_statement(
    function: &mut Function,
    statement: &WasmLirStmt,
    context: &LirBodyEmitContext<'_>,
    plan: &WasmEmitPlan,
) -> Result<(), CompilerError> {
    // WHAT: lower each LIR statement into explicit Wasm stack-machine instructions.
    // WHY: statement lowering is the only place that maps semantic LIR ops to concrete opcodes.
    match statement {
        WasmLirStmt::ConstI32 { dst, value } => {
            function.instruction(&Instruction::I32Const(*value));
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::ConstI64 { dst, value } => {
            function.instruction(&Instruction::I64Const(*value));
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::ConstF32 { dst, value } => {
            function.instruction(&Instruction::F32Const((*value).into()));
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::ConstF64 { dst, value } => {
            function.instruction(&Instruction::F64Const((*value).into()));
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::ConstStaticPtr { dst, data } => {
            let offset = plan.data_offsets.get(data).copied().ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Wasm emission missing offset for static data {:?} in function {:?}",
                    data, context.function_id
                ))
                .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
            })?;
            function.instruction(&Instruction::I32Const(offset as i32));
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::ConstLength { dst, value } => {
            if *value > i32::MAX as u32 {
                return Err(CompilerError::compiler_error(format!(
                    "Wasm emission cannot lower length {} > i32::MAX in function {:?}",
                    value, context.function_id
                ))
                .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
            }
            function.instruction(&Instruction::I32Const(*value as i32));
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::Copy { dst, src } | WasmLirStmt::Move { dst, src } => {
            // WHAT: the current emitter models copy/move as local assignment in emitted Wasm.
            // WHY: ownership specialization remains in runtime/helper behavior for now.
            function.instruction(&Instruction::LocalGet(local_index(*src, context)?));
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::Call { dst, callee, args } => {
            for arg in args {
                function.instruction(&Instruction::LocalGet(local_index(*arg, context)?));
            }

            let callee_index = match callee {
                WasmCalleeRef::Function(function_id) => plan
                    .function_indices
                    .get(function_id)
                    .copied()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "Wasm emission missing callee function index for {:?} in {:?}",
                            function_id, context.function_id
                        ))
                        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
                    })?,
                WasmCalleeRef::Import(import_id) => plan
                    .import_function_indices
                    .get(import_id)
                    .copied()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "Wasm emission missing import function index for {:?} in {:?}",
                            import_id, context.function_id
                        ))
                        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
                    })?,
            };
            function.instruction(&Instruction::Call(callee_index));

            if let Some(dst) = dst {
                function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
            }
        }
        WasmLirStmt::StringNewBuffer { dst } => {
            function.instruction(&Instruction::Call(helper_index(
                plan,
                WasmRuntimeHelper::StringNewBuffer,
            )?));
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::StringPushLiteral { buffer, data } => {
            let ptr = plan.data_offsets.get(data).copied().ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Wasm emission missing static pointer for {:?} in {:?}",
                    data, context.function_id
                ))
                .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
            })?;
            let len = plan.data_lengths.get(data).copied().ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Wasm emission missing static length for {:?} in {:?}",
                    data, context.function_id
                ))
                .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
            })?;
            function.instruction(&Instruction::LocalGet(local_index(*buffer, context)?));
            function.instruction(&Instruction::I32Const(ptr as i32));
            function.instruction(&Instruction::I32Const(len as i32));
            function.instruction(&Instruction::Call(helper_index(
                plan,
                WasmRuntimeHelper::StringPushLiteral,
            )?));
        }
        WasmLirStmt::StringPushHandle { buffer, handle } => {
            function.instruction(&Instruction::LocalGet(local_index(*buffer, context)?));
            function.instruction(&Instruction::LocalGet(local_index(*handle, context)?));
            function.instruction(&Instruction::Call(helper_index(
                plan,
                WasmRuntimeHelper::StringPushHandle,
            )?));
        }
        WasmLirStmt::StringFromI64 { dst, value } => {
            function.instruction(&Instruction::LocalGet(local_index(*value, context)?));
            function.instruction(&Instruction::Call(helper_index(
                plan,
                WasmRuntimeHelper::StringFromI64,
            )?));
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::StringFinish { dst, buffer } => {
            function.instruction(&Instruction::LocalGet(local_index(*buffer, context)?));
            function.instruction(&Instruction::Call(helper_index(
                plan,
                WasmRuntimeHelper::StringFinish,
            )?));
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::VecNew { dst } => {
            function.instruction(&Instruction::Call(helper_index(
                plan,
                WasmRuntimeHelper::VecNew,
            )?));
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::VecPushHandle { vec, handle } => {
            function.instruction(&Instruction::LocalGet(local_index(*vec, context)?));
            function.instruction(&Instruction::LocalGet(local_index(*handle, context)?));
            function.instruction(&Instruction::Call(helper_index(
                plan,
                WasmRuntimeHelper::VecPushHandle,
            )?));
        }
        WasmLirStmt::DropIfOwned { value } => {
            function.instruction(&Instruction::LocalGet(local_index(*value, context)?));
            function.instruction(&Instruction::Call(helper_index(
                plan,
                WasmRuntimeHelper::DropIfOwned,
            )?));
        }
        WasmLirStmt::RetainHandle { .. } => {
            // WHAT: retain is currently a no-op at codegen level.
            // WHY: GC-first semantics and conservative helper runtime make extra retain
            // unnecessary in the current emitter.
        }
        WasmLirStmt::IntEq { dst, lhs, rhs } => {
            emit_compare(function, *lhs, *rhs, context, true)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::IntNe { dst, lhs, rhs } => {
            emit_compare(function, *lhs, *rhs, context, false)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::IntAdd { dst, lhs, rhs } => {
            emit_numeric_add(function, *lhs, *rhs, context, NumericAddKind::Int)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::IntSub { dst, lhs, rhs } => {
            emit_numeric_sub(function, *lhs, *rhs, context, NumericSubKind::Int)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::IntMod { dst, lhs, rhs } => {
            let dst_idx = local_index(*dst, context)?;
            let lhs_idx = local_index(*lhs, context)?;
            let rhs_idx = local_index(*rhs, context)?;

            // rem = lhs rem_s rhs  (signed/truncating remainder)
            function.instruction(&Instruction::LocalGet(lhs_idx));
            function.instruction(&Instruction::LocalGet(rhs_idx));
            function.instruction(&Instruction::I64RemS);
            function.instruction(&Instruction::LocalSet(dst_idx));

            // Euclidean correction: if rem < 0, add abs(rhs)
            function.instruction(&Instruction::LocalGet(dst_idx));
            function.instruction(&Instruction::I64Const(0));
            function.instruction(&Instruction::I64LtS);
            function.instruction(&Instruction::If(BlockType::Empty));
            function.instruction(&Instruction::LocalGet(dst_idx));
            // Compute abs(rhs): rhs < 0 ? (0 - rhs) : rhs
            function.instruction(&Instruction::LocalGet(rhs_idx));
            function.instruction(&Instruction::I64Const(0));
            function.instruction(&Instruction::I64LtS);
            function.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            function.instruction(&Instruction::I64Const(0));
            function.instruction(&Instruction::LocalGet(rhs_idx));
            function.instruction(&Instruction::I64Sub);
            function.instruction(&Instruction::Else);
            function.instruction(&Instruction::LocalGet(rhs_idx));
            function.instruction(&Instruction::End);
            function.instruction(&Instruction::I64Add);
            function.instruction(&Instruction::LocalSet(dst_idx));
            function.instruction(&Instruction::End);
        }
        WasmLirStmt::IntMul { dst, lhs, rhs } => {
            emit_numeric_mul(function, *lhs, *rhs, context, NumericMulKind::Int)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::IntFloorDiv { dst, lhs, rhs } => {
            function.instruction(&Instruction::LocalGet(local_index(*lhs, context)?));
            function.instruction(&Instruction::LocalGet(local_index(*rhs, context)?));
            function.instruction(&Instruction::I64DivS);
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::IntToFloatDiv { dst, lhs, rhs } => {
            // Convert I64 operands to F64, then divide.
            // WHY: Moth Int / Int always yields Float; conversion is the emitter's responsibility.
            function.instruction(&Instruction::LocalGet(local_index(*lhs, context)?));
            function.instruction(&Instruction::F64ConvertI64S);
            function.instruction(&Instruction::LocalGet(local_index(*rhs, context)?));
            function.instruction(&Instruction::F64ConvertI64S);
            function.instruction(&Instruction::F64Div);
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::FloatAdd { dst, lhs, rhs } => {
            emit_numeric_add(function, *lhs, *rhs, context, NumericAddKind::Float)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::FloatSub { dst, lhs, rhs } => {
            emit_numeric_sub(function, *lhs, *rhs, context, NumericSubKind::Float)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::FloatMul { dst, lhs, rhs } => {
            emit_numeric_mul(function, *lhs, *rhs, context, NumericMulKind::Float)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::FloatDiv { dst, lhs, rhs } => {
            emit_numeric_div(function, *lhs, *rhs, context)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::FloatMod { dst, lhs, rhs } => {
            let dst_idx = local_index(*dst, context)?;
            let lhs_idx = local_index(*lhs, context)?;
            let rhs_idx = local_index(*rhs, context)?;
            let lhs_type = local_type(*lhs, context, "lhs")?;

            // Euclidean: a − b·floor(a/b) using the WASM value stack.
            // Stack trace: [a][a][b] → [a][a/b] → [a][floor(a/b)][b] → [a][floor(a/b)·b] → [result]
            function.instruction(&Instruction::LocalGet(lhs_idx));
            function.instruction(&Instruction::LocalGet(lhs_idx));
            function.instruction(&Instruction::LocalGet(rhs_idx));
            match lhs_type {
                WasmAbiType::F32 => {
                    function.instruction(&Instruction::F32Div);
                    function.instruction(&Instruction::F32Floor);
                    function.instruction(&Instruction::LocalGet(rhs_idx));
                    function.instruction(&Instruction::F32Mul);
                    function.instruction(&Instruction::F32Sub);
                }
                WasmAbiType::F64 => {
                    function.instruction(&Instruction::F64Div);
                    function.instruction(&Instruction::F64Floor);
                    function.instruction(&Instruction::LocalGet(rhs_idx));
                    function.instruction(&Instruction::F64Mul);
                    function.instruction(&Instruction::F64Sub);
                }
                other => {
                    return Err(CompilerError::compiler_error(format!(
                        "Wasm emission FloatMod requires F32 or F64 operands, found {other:?} in {:?}",
                        context.function_id
                    ))
                    .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
                }
            }
            function.instruction(&Instruction::LocalSet(dst_idx));
        }
        WasmLirStmt::BoolAnd { dst, lhs, rhs } => {
            function.instruction(&Instruction::LocalGet(local_index(*lhs, context)?));
            function.instruction(&Instruction::LocalGet(local_index(*rhs, context)?));
            function.instruction(&Instruction::I32And);
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::BoolOr { dst, lhs, rhs } => {
            function.instruction(&Instruction::LocalGet(local_index(*lhs, context)?));
            function.instruction(&Instruction::LocalGet(local_index(*rhs, context)?));
            function.instruction(&Instruction::I32Or);
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::OrderedLt { dst, lhs, rhs } => {
            emit_ordered_compare(function, *lhs, *rhs, context, OrderedCompareKind::Lt)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::OrderedLe { dst, lhs, rhs } => {
            emit_ordered_compare(function, *lhs, *rhs, context, OrderedCompareKind::Le)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::OrderedGt { dst, lhs, rhs } => {
            emit_ordered_compare(function, *lhs, *rhs, context, OrderedCompareKind::Gt)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::OrderedGe { dst, lhs, rhs } => {
            emit_ordered_compare(function, *lhs, *rhs, context, OrderedCompareKind::Ge)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
    }

    Ok(())
}

pub(crate) fn emit_terminator(
    function: &mut Function,
    terminator: &WasmLirTerminator,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    match terminator {
        WasmLirTerminator::Jump(target) => {
            set_dispatch_target(function, *target, context)?;
            // Branch depth 1 exits the current `if` and restarts the dispatcher loop.
            function.instruction(&Instruction::Br(1));
        }
        WasmLirTerminator::Branch {
            condition,
            then_block,
            else_block,
        } => {
            function.instruction(&Instruction::LocalGet(local_index(*condition, context)?));
            function.instruction(&Instruction::If(BlockType::Empty));
            set_dispatch_target(function, *then_block, context)?;
            function.instruction(&Instruction::Else);
            set_dispatch_target(function, *else_block, context)?;
            function.instruction(&Instruction::End);
            // Re-enter the loop with the newly selected target block.
            function.instruction(&Instruction::Br(1));
        }
        WasmLirTerminator::Return { value } => {
            if let Some(value) = value {
                function.instruction(&Instruction::LocalGet(local_index(*value, context)?));
            }
            function.instruction(&Instruction::Return);
        }
        WasmLirTerminator::Trap => {
            function.instruction(&Instruction::Unreachable);
        }
    }

    Ok(())
}

fn set_dispatch_target(
    function: &mut Function,
    target: WasmLirBlockId,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    // WHAT: convert target block id into dispatcher-table index.
    // WHY: branch/jump terminators communicate control flow via dispatch-local updates.
    let target_index = context
        .block_index_by_id
        .get(&target)
        .copied()
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Wasm emission missing block index for target {:?} in function {:?}",
                target, context.function_id
            ))
            .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
        })?;

    function.instruction(&Instruction::I32Const(target_index as i32));
    function.instruction(&Instruction::LocalSet(context.dispatch_local_index));
    Ok(())
}

#[derive(Clone, Copy)]
enum NumericAddKind {
    Int,
    Float,
}

#[derive(Clone, Copy)]
enum NumericSubKind {
    Int,
    Float,
}

#[derive(Clone, Copy)]
enum OrderedCompareKind {
    Lt,
    Le,
    Gt,
    Ge,
}

fn emit_compare(
    function: &mut Function,
    lhs: WasmLirLocalId,
    rhs: WasmLirLocalId,
    context: &LirBodyEmitContext<'_>,
    is_eq: bool,
) -> Result<(), CompilerError> {
    function.instruction(&Instruction::LocalGet(local_index(lhs, context)?));
    function.instruction(&Instruction::LocalGet(local_index(rhs, context)?));

    let lhs_type = local_type(lhs, context, "lhs")?;
    let rhs_type = local_type(rhs, context, "rhs")?;
    if lhs_type != rhs_type {
        return Err(CompilerError::compiler_error(format!(
            "Wasm emission type mismatch in comparison: lhs {:?} is {:?}, rhs {:?} is {:?} in {:?}",
            lhs, lhs_type, rhs, rhs_type, context.function_id
        ))
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
    }

    match lhs_type {
        WasmAbiType::I64 => {
            function.instruction(if is_eq {
                &Instruction::I64Eq
            } else {
                &Instruction::I64Ne
            });
        }
        WasmAbiType::F32 => {
            function.instruction(if is_eq {
                &Instruction::F32Eq
            } else {
                &Instruction::F32Ne
            });
        }
        WasmAbiType::F64 => {
            function.instruction(if is_eq {
                &Instruction::F64Eq
            } else {
                &Instruction::F64Ne
            });
        }
        WasmAbiType::Void => {
            return Err(CompilerError::compiler_error(format!(
                "Wasm emission cannot compare Void-typed locals in function {:?}",
                context.function_id
            ))
            .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
        }
        WasmAbiType::I32 | WasmAbiType::Handle => {
            function.instruction(if is_eq {
                &Instruction::I32Eq
            } else {
                &Instruction::I32Ne
            });
        }
    }

    Ok(())
}

fn emit_numeric_add(
    function: &mut Function,
    lhs: WasmLirLocalId,
    rhs: WasmLirLocalId,
    context: &LirBodyEmitContext<'_>,
    kind: NumericAddKind,
) -> Result<(), CompilerError> {
    function.instruction(&Instruction::LocalGet(local_index(lhs, context)?));
    function.instruction(&Instruction::LocalGet(local_index(rhs, context)?));

    let lhs_type = local_type(lhs, context, "lhs")?;
    let rhs_type = local_type(rhs, context, "rhs")?;
    if lhs_type != rhs_type {
        return Err(CompilerError::compiler_error(format!(
            "Wasm emission type mismatch in numeric add: lhs {:?} is {:?}, rhs {:?} is {:?} in {:?}",
            lhs, lhs_type, rhs, rhs_type, context.function_id
        ))
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
    }

    match kind {
        NumericAddKind::Int => match lhs_type {
            WasmAbiType::I64 => {
                function.instruction(&Instruction::I64Add);
            }
            _ => {
                return Err(CompilerError::compiler_error(format!(
                    "Wasm emission cannot lower IntAdd for ABI type {:?} in function {:?}",
                    lhs_type, context.function_id
                ))
                .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
            }
        },
        NumericAddKind::Float => match lhs_type {
            WasmAbiType::F32 => {
                function.instruction(&Instruction::F32Add);
            }
            WasmAbiType::F64 => {
                function.instruction(&Instruction::F64Add);
            }
            _ => {
                return Err(CompilerError::compiler_error(format!(
                    "Wasm emission cannot lower FloatAdd for ABI type {:?} in function {:?}",
                    lhs_type, context.function_id
                ))
                .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
            }
        },
    }

    Ok(())
}

fn emit_ordered_compare(
    function: &mut Function,
    lhs: WasmLirLocalId,
    rhs: WasmLirLocalId,
    context: &LirBodyEmitContext<'_>,
    kind: OrderedCompareKind,
) -> Result<(), CompilerError> {
    function.instruction(&Instruction::LocalGet(local_index(lhs, context)?));
    function.instruction(&Instruction::LocalGet(local_index(rhs, context)?));

    let lhs_type = local_type(lhs, context, "lhs")?;
    let rhs_type = local_type(rhs, context, "rhs")?;
    if lhs_type != rhs_type {
        return Err(CompilerError::compiler_error(format!(
            "Wasm emission type mismatch in ordered comparison: lhs {:?} is {:?}, rhs {:?} is {:?} in {:?}",
            lhs, lhs_type, rhs, rhs_type, context.function_id
        ))
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
    }

    match lhs_type {
        WasmAbiType::I32 => {
            function.instruction(match kind {
                OrderedCompareKind::Lt => &Instruction::I32LtS,
                OrderedCompareKind::Le => &Instruction::I32LeS,
                OrderedCompareKind::Gt => &Instruction::I32GtS,
                OrderedCompareKind::Ge => &Instruction::I32GeS,
            });
        }
        WasmAbiType::I64 => {
            function.instruction(match kind {
                OrderedCompareKind::Lt => &Instruction::I64LtS,
                OrderedCompareKind::Le => &Instruction::I64LeS,
                OrderedCompareKind::Gt => &Instruction::I64GtS,
                OrderedCompareKind::Ge => &Instruction::I64GeS,
            });
        }
        WasmAbiType::F32 => {
            function.instruction(match kind {
                OrderedCompareKind::Lt => &Instruction::F32Lt,
                OrderedCompareKind::Le => &Instruction::F32Le,
                OrderedCompareKind::Gt => &Instruction::F32Gt,
                OrderedCompareKind::Ge => &Instruction::F32Ge,
            });
        }
        WasmAbiType::F64 => {
            function.instruction(match kind {
                OrderedCompareKind::Lt => &Instruction::F64Lt,
                OrderedCompareKind::Le => &Instruction::F64Le,
                OrderedCompareKind::Gt => &Instruction::F64Gt,
                OrderedCompareKind::Ge => &Instruction::F64Ge,
            });
        }
        WasmAbiType::Handle | WasmAbiType::Void => {
            return Err(CompilerError::compiler_error(format!(
                "Wasm emission cannot lower ordered comparison for ABI type {:?} in function {:?}",
                lhs_type, context.function_id
            ))
            .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
        }
    }

    Ok(())
}

fn emit_numeric_sub(
    function: &mut Function,
    lhs: WasmLirLocalId,
    rhs: WasmLirLocalId,
    context: &LirBodyEmitContext<'_>,
    kind: NumericSubKind,
) -> Result<(), CompilerError> {
    function.instruction(&Instruction::LocalGet(local_index(lhs, context)?));
    function.instruction(&Instruction::LocalGet(local_index(rhs, context)?));

    let lhs_type = local_type(lhs, context, "lhs")?;
    let rhs_type = local_type(rhs, context, "rhs")?;
    if lhs_type != rhs_type {
        return Err(CompilerError::compiler_error(format!(
            "Wasm emission type mismatch in numeric sub: lhs {:?} is {:?}, rhs {:?} is {:?} in {:?}",
            lhs, lhs_type, rhs, rhs_type, context.function_id
        ))
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
    }

    match kind {
        NumericSubKind::Int => match lhs_type {
            WasmAbiType::I64 => {
                function.instruction(&Instruction::I64Sub);
            }
            _ => {
                return Err(CompilerError::compiler_error(format!(
                    "Wasm emission cannot lower IntSub for ABI type {:?} in function {:?}",
                    lhs_type, context.function_id
                ))
                .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
            }
        },
        NumericSubKind::Float => match lhs_type {
            WasmAbiType::F32 => {
                function.instruction(&Instruction::F32Sub);
            }
            WasmAbiType::F64 => {
                function.instruction(&Instruction::F64Sub);
            }
            _ => {
                return Err(CompilerError::compiler_error(format!(
                    "Wasm emission cannot lower FloatSub for ABI type {:?} in function {:?}",
                    lhs_type, context.function_id
                ))
                .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
            }
        },
    }

    Ok(())
}

#[derive(Clone, Copy)]
enum NumericMulKind {
    Int,
    Float,
}

fn emit_numeric_mul(
    function: &mut Function,
    lhs: WasmLirLocalId,
    rhs: WasmLirLocalId,
    context: &LirBodyEmitContext<'_>,
    kind: NumericMulKind,
) -> Result<(), CompilerError> {
    function.instruction(&Instruction::LocalGet(local_index(lhs, context)?));
    function.instruction(&Instruction::LocalGet(local_index(rhs, context)?));

    let lhs_type = local_type(lhs, context, "lhs")?;
    let rhs_type = local_type(rhs, context, "rhs")?;
    if lhs_type != rhs_type {
        return Err(CompilerError::compiler_error(format!(
            "Wasm emission type mismatch in numeric mul: lhs {:?} is {:?}, rhs {:?} is {:?} in {:?}",
            lhs, lhs_type, rhs, rhs_type, context.function_id
        ))
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
    }

    match kind {
        NumericMulKind::Int => match lhs_type {
            WasmAbiType::I64 => {
                function.instruction(&Instruction::I64Mul);
            }
            _ => {
                return Err(CompilerError::compiler_error(format!(
                    "Wasm emission cannot lower IntMul for ABI type {:?} in function {:?}",
                    lhs_type, context.function_id
                ))
                .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
            }
        },
        NumericMulKind::Float => match lhs_type {
            WasmAbiType::F32 => {
                function.instruction(&Instruction::F32Mul);
            }
            WasmAbiType::F64 => {
                function.instruction(&Instruction::F64Mul);
            }
            _ => {
                return Err(CompilerError::compiler_error(format!(
                    "Wasm emission cannot lower FloatMul for ABI type {:?} in function {:?}",
                    lhs_type, context.function_id
                ))
                .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
            }
        },
    }

    Ok(())
}

fn emit_numeric_div(
    function: &mut Function,
    lhs: WasmLirLocalId,
    rhs: WasmLirLocalId,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    function.instruction(&Instruction::LocalGet(local_index(lhs, context)?));
    function.instruction(&Instruction::LocalGet(local_index(rhs, context)?));

    let lhs_type = local_type(lhs, context, "lhs")?;
    let rhs_type = local_type(rhs, context, "rhs")?;
    if lhs_type != rhs_type {
        return Err(CompilerError::compiler_error(format!(
            "Wasm emission type mismatch in float div: lhs {:?} is {:?}, rhs {:?} is {:?} in {:?}",
            lhs, lhs_type, rhs, rhs_type, context.function_id
        ))
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
    }

    match lhs_type {
        WasmAbiType::F32 => {
            function.instruction(&Instruction::F32Div);
        }
        WasmAbiType::F64 => {
            function.instruction(&Instruction::F64Div);
        }
        _ => {
            return Err(CompilerError::compiler_error(format!(
                "Wasm emission cannot lower FloatDiv for ABI type {:?} in function {:?}",
                lhs_type, context.function_id
            ))
            .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
        }
    }

    Ok(())
}

fn local_type(
    local_id: WasmLirLocalId,
    context: &LirBodyEmitContext<'_>,
    label: &str,
) -> Result<WasmAbiType, CompilerError> {
    context
        .local_type_by_id
        .get(&local_id)
        .copied()
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Wasm emission missing local type for {label} {:?} in function {:?}",
                local_id, context.function_id
            ))
            .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
        })
}

fn local_index(
    local_id: WasmLirLocalId,
    context: &LirBodyEmitContext<'_>,
) -> Result<u32, CompilerError> {
    // WHAT: resolve LIR local ids to final Wasm local indices.
    // WHY: keeping this lookup centralized guarantees consistent error diagnostics.
    context
        .local_index_by_id
        .get(&local_id)
        .copied()
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Wasm emission missing local index for {:?} in function {:?}",
                local_id, context.function_id
            ))
            .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
        })
}

fn helper_index(plan: &WasmEmitPlan, helper: WasmRuntimeHelper) -> Result<u32, CompilerError> {
    // WHAT: resolve synthesized helper to its planned function index.
    // WHY: helper calls are encoded as direct calls and must match plan indices exactly.
    plan.helper_indices.get(&helper).copied().ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "Wasm emission missing helper function index for {}",
            crate::backends::wasm::emit::sections::helper_name(helper)
        ))
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
    })
}
