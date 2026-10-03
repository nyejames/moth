//! Instruction lowering from Wasm LIR to wasm-encoder instructions.

use crate::backends::error_types::BackendErrorType;
use crate::backends::wasm::emit::sections::WasmEmitPlan;
use crate::backends::wasm::lir::instructions::{
    WasmCalleeRef, WasmLirStmt, WasmLirTerminator, WasmScalarComparisonOp, WasmScalarComparisonType,
};
use crate::backends::wasm::lir::types::{
    WasmAbiType, WasmLirBlockId, WasmLirFunctionId, WasmLirLocalId,
};
use crate::backends::wasm::runtime::memory::WasmScalarStorageKind;
use crate::backends::wasm::runtime::strings::WasmRuntimeHelper;
use crate::compiler_frontend::compiler_messages::compiler_errors::{CompilerError, ErrorType};
use moth_lexical::numeric::precision::BinaryFloatPrecision;
use rustc_hash::FxHashMap;
use wasm_encoder::{BlockType, Function, Instruction, MemArg, ValType};

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
            emit_integer_to_string(function, *dst, *value, context, plan, true)?;
        }
        WasmLirStmt::StringFromU64 { dst, value } => {
            emit_integer_to_string(function, *dst, *value, context, plan, false)?;
        }
        WasmLirStmt::StringFromFloat {
            dst,
            value,
            precision,
        } => {
            emit_float_to_string(function, *dst, *value, *precision, context, plan)?;
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
            // WHY: the current helper runtime uses transitional collected scaffolding, so this
            // reserved hook has no effect. Final lowering will derive retain traffic from the
            // validated memory plan rather than from a tracing-collector fallback.
        }
        WasmLirStmt::IntEq { dst, lhs, rhs } => {
            emit_compare(function, *lhs, *rhs, context, true)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::IntNe { dst, lhs, rhs } => {
            emit_compare(function, *lhs, *rhs, context, false)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::ScalarCompare {
            dst,
            lhs,
            rhs,
            op,
            lhs_type,
            rhs_type,
        } => {
            emit_scalar_compare(function, *lhs, *rhs, *op, *lhs_type, *rhs_type, context)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::StringEq { dst, lhs, rhs } => {
            emit_string_compare(function, *lhs, *rhs, context, plan, false)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::StringNe { dst, lhs, rhs } => {
            emit_string_compare(function, *lhs, *rhs, context, plan, true)?;
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::CheckedIntegerOp { operation } => {
            super::checked_integer::emit_checked_integer_operation(function, operation, context)?;
        }
        WasmLirStmt::ValidateFloat {
            dst,
            source,
            precision,
        } => {
            super::checked_float::emit_validate_float(
                function, *dst, *source, *precision, context,
            )?;
        }
        WasmLirStmt::CheckedFloatOp {
            dst,
            operator,
            precision,
            operands,
        } => {
            super::checked_float::emit_checked_float_operation(
                function, *dst, *operator, *precision, *operands, context, plan,
            )?;
        }
        WasmLirStmt::FloatRangeCandidate {
            candidate_dst,
            in_range_dst,
            scratch,
            current,
            step,
            end,
            ascending,
            precision,
            inclusive,
        } => {
            emit_float_range_candidate(
                function,
                [*current, *step, *end, *ascending],
                [*candidate_dst, *in_range_dst, *scratch],
                *precision,
                *inclusive,
                context,
            )?;
        }
        WasmLirStmt::IntegerToFloat {
            dst,
            source,
            source_signed,
        } => {
            let source_type = local_type(*source, context, "IntegerToFloat source")?;
            let dst_type = local_type(*dst, context, "IntegerToFloat destination")?;
            let conversion = match (source_type, dst_type, source_signed) {
                (WasmAbiType::I32, WasmAbiType::F32, true) => &Instruction::F32ConvertI32S,
                (WasmAbiType::I32, WasmAbiType::F32, false) => &Instruction::F32ConvertI32U,
                (WasmAbiType::I64, WasmAbiType::F32, true) => &Instruction::F32ConvertI64S,
                (WasmAbiType::I64, WasmAbiType::F32, false) => &Instruction::F32ConvertI64U,
                (WasmAbiType::I32, WasmAbiType::F64, true) => &Instruction::F64ConvertI32S,
                (WasmAbiType::I32, WasmAbiType::F64, false) => &Instruction::F64ConvertI32U,
                (WasmAbiType::I64, WasmAbiType::F64, true) => &Instruction::F64ConvertI64S,
                (WasmAbiType::I64, WasmAbiType::F64, false) => &Instruction::F64ConvertI64U,
                (source_type, dst_type, _) => {
                    return Err(wasm_generation_error(format!(
                        "Wasm IntegerToFloat requires an integer source and F32/F64 destination, found {source_type:?} -> {dst_type:?}"
                    )));
                }
            };
            function.instruction(&Instruction::LocalGet(local_index(*source, context)?));
            function.instruction(conversion);
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::RoundF16 { dst, source } => {
            ensure_local_abi(*source, WasmAbiType::F32, context, "F16 rounding source")?;
            ensure_local_abi(*dst, WasmAbiType::F32, context, "F16 rounding destination")?;
            function.instruction(&Instruction::LocalGet(local_index(*source, context)?));
            function.instruction(&Instruction::Call(helper_index(
                plan,
                WasmRuntimeHelper::F32ToF16Bits,
            )?));
            function.instruction(&Instruction::Call(helper_index(
                plan,
                WasmRuntimeHelper::F16BitsToF32,
            )?));
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::FloatExtend { dst, source } => {
            ensure_local_abi(*source, WasmAbiType::F32, context, "float extension source")?;
            ensure_local_abi(
                *dst,
                WasmAbiType::F64,
                context,
                "float extension destination",
            )?;
            function.instruction(&Instruction::LocalGet(local_index(*source, context)?));
            function.instruction(&Instruction::F64PromoteF32);
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::IntegerExtend {
            dst,
            source,
            source_signed,
        } => {
            ensure_local_abi(
                *source,
                WasmAbiType::I32,
                context,
                "integer extension source",
            )?;
            ensure_local_abi(
                *dst,
                WasmAbiType::I64,
                context,
                "integer extension destination",
            )?;
            function.instruction(&Instruction::LocalGet(local_index(*source, context)?));
            function.instruction(if *source_signed {
                &Instruction::I64ExtendI32S
            } else {
                &Instruction::I64ExtendI32U
            });
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::LoadScalar {
            dst,
            address,
            offset,
            kind,
        } => {
            ensure_local_abi(*address, WasmAbiType::I32, context, "scalar load address")?;
            ensure_local_abi(*dst, kind.carrier(), context, "scalar load result")?;
            function.instruction(&Instruction::LocalGet(local_index(*address, context)?));
            function.instruction(&scalar_load_instruction(*kind, *offset));
            if *kind == WasmScalarStorageKind::F16 {
                function.instruction(&Instruction::Call(helper_index(
                    plan,
                    WasmRuntimeHelper::F16BitsToF32,
                )?));
            }
            function.instruction(&Instruction::LocalSet(local_index(*dst, context)?));
        }
        WasmLirStmt::StoreScalar {
            address,
            offset,
            value,
            kind,
        } => {
            ensure_local_abi(*address, WasmAbiType::I32, context, "scalar store address")?;
            ensure_local_abi(*value, kind.carrier(), context, "scalar store value")?;
            function.instruction(&Instruction::LocalGet(local_index(*address, context)?));
            function.instruction(&Instruction::LocalGet(local_index(*value, context)?));
            if *kind == WasmScalarStorageKind::F16 {
                function.instruction(&Instruction::Call(helper_index(
                    plan,
                    WasmRuntimeHelper::F32ToF16Bits,
                )?));
            }
            function.instruction(&scalar_store_instruction(*kind, *offset));
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

fn emit_float_range_candidate(
    function: &mut Function,
    inputs: [WasmLirLocalId; 4],
    output_ids: [WasmLirLocalId; 3],
    precision: BinaryFloatPrecision,
    inclusive: bool,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    let [current, step, end, ascending] = inputs;
    let [candidate_dst, in_range_dst, scratch] = output_ids;
    let carrier = match precision {
        BinaryFloatPrecision::Binary32 => WasmAbiType::F32,
        BinaryFloatPrecision::Binary64 => WasmAbiType::F64,
        BinaryFloatPrecision::Binary16 => {
            return Err(wasm_generation_error(
                "Wasm float range candidates do not support F16".to_owned(),
            ));
        }
    };
    ensure_local_abi(current, carrier, context, "float range current")?;
    ensure_local_abi(step, carrier, context, "float range step")?;
    ensure_local_abi(end, carrier, context, "float range end")?;
    ensure_local_abi(scratch, carrier, context, "float range candidate scratch")?;
    ensure_local_abi(
        ascending,
        WasmAbiType::I32,
        context,
        "float range direction",
    )?;
    ensure_local_abi(
        in_range_dst,
        WasmAbiType::I32,
        context,
        "float range Bool result",
    )?;
    ensure_local_abi(
        candidate_dst,
        carrier,
        context,
        "float range candidate destination",
    )?;

    let (add_opcode, subtract_opcode, ascending_compare, descending_compare) = match precision {
        BinaryFloatPrecision::Binary32 => (
            &Instruction::F32Add,
            &Instruction::F32Sub,
            if inclusive {
                &Instruction::F32Le
            } else {
                &Instruction::F32Lt
            },
            if inclusive {
                &Instruction::F32Ge
            } else {
                &Instruction::F32Gt
            },
        ),
        BinaryFloatPrecision::Binary64 => (
            &Instruction::F64Add,
            &Instruction::F64Sub,
            if inclusive {
                &Instruction::F64Le
            } else {
                &Instruction::F64Lt
            },
            if inclusive {
                &Instruction::F64Ge
            } else {
                &Instruction::F64Gt
            },
        ),
        BinaryFloatPrecision::Binary16 => unreachable!("F16 precision was rejected above"),
    };

    function.instruction(&Instruction::LocalGet(local_index(ascending, context)?));
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(local_index(current, context)?));
    function.instruction(&Instruction::LocalGet(local_index(step, context)?));
    function.instruction(add_opcode);
    function.instruction(&Instruction::LocalSet(local_index(scratch, context)?));
    function.instruction(&Instruction::Else);
    function.instruction(&Instruction::LocalGet(local_index(current, context)?));
    function.instruction(&Instruction::LocalGet(local_index(step, context)?));
    function.instruction(subtract_opcode);
    function.instruction(&Instruction::LocalSet(local_index(scratch, context)?));
    function.instruction(&Instruction::End);

    // Native F32/F64 operations already rounded into scratch. Keep the finite predicate separate
    // from ordinary checked arithmetic so an out-of-range infinity exits without trapping.
    function.instruction(&Instruction::LocalGet(local_index(scratch, context)?));
    super::checked_float::emit_finite_float_predicate(function, precision)?;
    function.instruction(&Instruction::LocalGet(local_index(ascending, context)?));
    function.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    function.instruction(&Instruction::LocalGet(local_index(scratch, context)?));
    function.instruction(&Instruction::LocalGet(local_index(end, context)?));
    function.instruction(ascending_compare);
    function.instruction(&Instruction::Else);
    function.instruction(&Instruction::LocalGet(local_index(scratch, context)?));
    function.instruction(&Instruction::LocalGet(local_index(end, context)?));
    function.instruction(descending_compare);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::LocalSet(local_index(in_range_dst, context)?));

    function.instruction(&Instruction::LocalGet(local_index(in_range_dst, context)?));
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(local_index(scratch, context)?));
    function.instruction(&Instruction::LocalSet(local_index(candidate_dst, context)?));
    function.instruction(&Instruction::End);

    Ok(())
}

fn emit_integer_to_string(
    function: &mut Function,
    dst: WasmLirLocalId,
    value: WasmLirLocalId,
    context: &LirBodyEmitContext<'_>,
    plan: &WasmEmitPlan,
    signed: bool,
) -> Result<(), CompilerError> {
    let operation = if signed {
        "StringFromI64"
    } else {
        "StringFromU64"
    };
    ensure_local_abi(dst, WasmAbiType::Handle, context, operation)?;
    let value_type = local_type(value, context, operation)?;
    function.instruction(&Instruction::LocalGet(local_index(value, context)?));
    match (value_type, signed) {
        (WasmAbiType::I32, true) => {
            function.instruction(&Instruction::I64ExtendI32S);
        }
        (WasmAbiType::I32, false) => {
            function.instruction(&Instruction::I64ExtendI32U);
        }
        (WasmAbiType::I64, _) => {}
        (other, _) => {
            return Err(wasm_generation_error(format!(
                "Wasm {operation} requires an I32 or I64 integer carrier, found {other:?}"
            )));
        }
    }
    let helper = if signed {
        WasmRuntimeHelper::StringFromI64
    } else {
        WasmRuntimeHelper::StringFromU64
    };
    function.instruction(&Instruction::Call(helper_index(plan, helper)?));
    function.instruction(&Instruction::LocalSet(local_index(dst, context)?));
    Ok(())
}

fn emit_float_to_string(
    function: &mut Function,
    dst: WasmLirLocalId,
    value: WasmLirLocalId,
    precision: BinaryFloatPrecision,
    context: &LirBodyEmitContext<'_>,
    plan: &WasmEmitPlan,
) -> Result<(), CompilerError> {
    let (value_abi, selector) = match precision {
        BinaryFloatPrecision::Binary16 => (WasmAbiType::F32, 16),
        BinaryFloatPrecision::Binary32 => (WasmAbiType::F32, 32),
        BinaryFloatPrecision::Binary64 => (WasmAbiType::F64, 64),
    };
    ensure_local_abi(
        dst,
        WasmAbiType::Handle,
        context,
        "StringFromFloat destination",
    )?;
    ensure_local_abi(value, value_abi, context, "StringFromFloat source")?;

    function.instruction(&Instruction::LocalGet(local_index(value, context)?));
    if value_abi == WasmAbiType::F32 {
        function.instruction(&Instruction::F64PromoteF32);
    }
    function.instruction(&Instruction::I32Const(selector));
    function.instruction(&Instruction::Call(helper_index(
        plan,
        WasmRuntimeHelper::StringFromFloat,
    )?));
    function.instruction(&Instruction::LocalSet(local_index(dst, context)?));
    Ok(())
}

// Wasm's memarg alignment is an access hint; the storage kind separately owns layout alignment.
fn scalar_memarg(kind: WasmScalarStorageKind, offset: u32) -> MemArg {
    MemArg {
        offset: u64::from(offset),
        align: kind.alignment().trailing_zeros(),
        memory_index: 0,
    }
}

fn scalar_load_instruction(kind: WasmScalarStorageKind, offset: u32) -> Instruction<'static> {
    let memarg = scalar_memarg(kind, offset);
    match kind {
        WasmScalarStorageKind::I8 => Instruction::I32Load8S(memarg),
        WasmScalarStorageKind::U8 => Instruction::I32Load8U(memarg),
        WasmScalarStorageKind::I16 => Instruction::I32Load16S(memarg),
        WasmScalarStorageKind::U16 | WasmScalarStorageKind::F16 => Instruction::I32Load16U(memarg),
        WasmScalarStorageKind::I32 | WasmScalarStorageKind::U32 => Instruction::I32Load(memarg),
        WasmScalarStorageKind::I64 | WasmScalarStorageKind::U64 => Instruction::I64Load(memarg),
        WasmScalarStorageKind::F32 => Instruction::F32Load(memarg),
        WasmScalarStorageKind::F64 => Instruction::F64Load(memarg),
    }
}

fn scalar_store_instruction(kind: WasmScalarStorageKind, offset: u32) -> Instruction<'static> {
    let memarg = scalar_memarg(kind, offset);
    match kind {
        WasmScalarStorageKind::I8 | WasmScalarStorageKind::U8 => Instruction::I32Store8(memarg),
        WasmScalarStorageKind::I16 | WasmScalarStorageKind::U16 | WasmScalarStorageKind::F16 => {
            Instruction::I32Store16(memarg)
        }
        WasmScalarStorageKind::I32 | WasmScalarStorageKind::U32 => Instruction::I32Store(memarg),
        WasmScalarStorageKind::I64 | WasmScalarStorageKind::U64 => Instruction::I64Store(memarg),
        WasmScalarStorageKind::F32 => Instruction::F32Store(memarg),
        WasmScalarStorageKind::F64 => Instruction::F64Store(memarg),
    }
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

fn emit_string_compare(
    function: &mut Function,
    lhs: WasmLirLocalId,
    rhs: WasmLirLocalId,
    context: &LirBodyEmitContext<'_>,
    plan: &WasmEmitPlan,
    negate: bool,
) -> Result<(), CompilerError> {
    let lhs_type = local_type(lhs, context, "lhs")?;
    let rhs_type = local_type(rhs, context, "rhs")?;
    if lhs_type != WasmAbiType::Handle || rhs_type != WasmAbiType::Handle {
        return Err(CompilerError::compiler_error(format!(
            "Wasm emission String comparison requires Handle operands, found lhs {:?}, rhs {:?} in {:?}",
            lhs_type, rhs_type, context.function_id
        ))
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
    }

    function.instruction(&Instruction::LocalGet(local_index(lhs, context)?));
    function.instruction(&Instruction::LocalGet(local_index(rhs, context)?));
    function.instruction(&Instruction::Call(helper_index(
        plan,
        WasmRuntimeHelper::StringEqual,
    )?));
    if negate {
        function.instruction(&Instruction::I32Eqz);
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
        WasmAbiType::I32 | WasmAbiType::I64 => {
            emit_integer_ordered_opcode(function, lhs_type, kind, false);
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

fn emit_scalar_compare(
    function: &mut Function,
    lhs: WasmLirLocalId,
    rhs: WasmLirLocalId,
    op: WasmScalarComparisonOp,
    lhs_type: WasmScalarComparisonType,
    rhs_type: WasmScalarComparisonType,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    let lhs_integer = integer_scalar_shape(lhs_type).is_some();
    let rhs_integer = integer_scalar_shape(rhs_type).is_some();
    let lhs_float = matches!(lhs_type, WasmScalarComparisonType::Float(_));
    let rhs_float = matches!(rhs_type, WasmScalarComparisonType::Float(_));

    match (lhs_integer, rhs_integer, lhs_float, rhs_float) {
        (true, true, false, false) => {
            emit_integer_compare(function, lhs, rhs, lhs_type, rhs_type, op, context)
        }
        (false, false, true, true) => {
            if lhs_type == rhs_type
                && matches!(
                    lhs_type,
                    WasmScalarComparisonType::Float(32) | WasmScalarComparisonType::Float(64)
                )
            {
                let bits = emit_float_as_native(function, lhs, lhs_type, context)?;
                emit_float_as_native(function, rhs, rhs_type, context)?;
                return emit_float_compare(function, bits, op, context);
            }

            emit_float_as_f64(function, lhs, lhs_type, context)?;
            emit_float_as_f64(function, rhs, rhs_type, context)?;
            emit_float_compare(function, 64, op, context)
        }
        _ => Err(wasm_generation_error(format!(
            "Wasm scalar comparison received incompatible operand types {lhs_type:?} and {rhs_type:?} in {:?}",
            context.function_id
        ))),
    }
}

fn emit_integer_compare(
    function: &mut Function,
    lhs: WasmLirLocalId,
    rhs: WasmLirLocalId,
    lhs_type: WasmScalarComparisonType,
    rhs_type: WasmScalarComparisonType,
    op: WasmScalarComparisonOp,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    let (lhs_bits, lhs_signed) = integer_scalar_shape(lhs_type).ok_or_else(|| {
        wasm_generation_error(format!(
            "Wasm integer comparison received non-integer lhs {lhs_type:?}"
        ))
    })?;
    let (rhs_bits, rhs_signed) = integer_scalar_shape(rhs_type).ok_or_else(|| {
        wasm_generation_error(format!(
            "Wasm integer comparison received non-integer rhs {rhs_type:?}"
        ))
    })?;
    let lhs_abi = integer_abi(lhs_bits, lhs_type, context)?;
    let rhs_abi = integer_abi(rhs_bits, rhs_type, context)?;

    // Equal signedness lets same-carrier values use the native opcode without widening.
    if lhs_signed == rhs_signed && lhs_abi == rhs_abi {
        ensure_local_abi(lhs, lhs_abi, context, "integer comparison")?;
        ensure_local_abi(rhs, rhs_abi, context, "integer comparison")?;
        function.instruction(&Instruction::LocalGet(local_index(lhs, context)?));
        function.instruction(&Instruction::LocalGet(local_index(rhs, context)?));
        emit_integer_compare_opcode(function, lhs_abi, op, !lhs_signed);
        return Ok(());
    }

    let mixed_u64 = lhs_signed != rhs_signed
        && ((!lhs_signed && lhs_bits == 64) || (!rhs_signed && rhs_bits == 64));

    if mixed_u64 && matches!(op, WasmScalarComparisonOp::Eq | WasmScalarComparisonOp::Ne) {
        let (signed_local, signed_type, unsigned_local, unsigned_type) = if lhs_signed {
            (lhs, lhs_type, rhs, rhs_type)
        } else {
            (rhs, rhs_type, lhs, lhs_type)
        };
        emit_integer_as_i64(function, signed_local, signed_type, context)?;
        function.instruction(&Instruction::I64Const(0));
        function.instruction(&Instruction::I64LtS);
        if op == WasmScalarComparisonOp::Eq {
            function.instruction(&Instruction::I32Eqz);
        }
        emit_integer_as_i64(function, signed_local, signed_type, context)?;
        emit_integer_as_i64(function, unsigned_local, unsigned_type, context)?;
        emit_integer_compare_opcode(function, WasmAbiType::I64, op, false);
        function.instruction(if op == WasmScalarComparisonOp::Eq {
            &Instruction::I32And
        } else {
            &Instruction::I32Or
        });
        return Ok(());
    }

    if mixed_u64 {
        let (signed_local, signed_type, unsigned_local, unsigned_type, signed_is_left) =
            if lhs_signed {
                (lhs, lhs_type, rhs, rhs_type, true)
            } else {
                (rhs, rhs_type, lhs, lhs_type, false)
            };
        let signed_op = if signed_is_left {
            op
        } else {
            invert_ordered_op(op)?
        };
        emit_signed_u64_ordered_compare(
            function,
            signed_local,
            signed_type,
            unsigned_local,
            unsigned_type,
            signed_op,
            context,
        )?;
        return Ok(());
    }

    emit_integer_as_i64(function, lhs, lhs_type, context)?;
    emit_integer_as_i64(function, rhs, rhs_type, context)?;
    emit_integer_compare_opcode(function, WasmAbiType::I64, op, !lhs_signed && !rhs_signed);
    Ok(())
}

fn emit_signed_u64_ordered_compare(
    function: &mut Function,
    signed_local: WasmLirLocalId,
    signed_type: WasmScalarComparisonType,
    unsigned_local: WasmLirLocalId,
    unsigned_type: WasmScalarComparisonType,
    op: WasmScalarComparisonOp,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    emit_integer_as_i64(function, signed_local, signed_type, context)?;
    function.instruction(&Instruction::I64Const(0));
    function.instruction(&Instruction::I64LtS);
    if matches!(op, WasmScalarComparisonOp::Gt | WasmScalarComparisonOp::Ge) {
        function.instruction(&Instruction::I32Eqz);
    }

    emit_integer_as_i64(function, signed_local, signed_type, context)?;
    emit_integer_as_i64(function, unsigned_local, unsigned_type, context)?;
    emit_integer_ordered_opcode(function, WasmAbiType::I64, ordered_compare_kind(op), true);
    function.instruction(
        if matches!(op, WasmScalarComparisonOp::Lt | WasmScalarComparisonOp::Le) {
            &Instruction::I32Or
        } else {
            &Instruction::I32And
        },
    );
    Ok(())
}

fn emit_integer_as_i64(
    function: &mut Function,
    local: WasmLirLocalId,
    scalar_type: WasmScalarComparisonType,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    let (bits, signed) = integer_scalar_shape(scalar_type).ok_or_else(|| {
        wasm_generation_error(format!(
            "Wasm expected an integer scalar type, found {scalar_type:?} in {:?}",
            context.function_id
        ))
    })?;
    let expected_abi = integer_abi(bits, scalar_type, context)?;
    ensure_local_abi(local, expected_abi, context, "integer comparison")?;
    function.instruction(&Instruction::LocalGet(local_index(local, context)?));
    if bits <= 32 {
        function.instruction(if signed {
            &Instruction::I64ExtendI32S
        } else {
            &Instruction::I64ExtendI32U
        });
    }
    Ok(())
}

fn emit_float_as_native(
    function: &mut Function,
    local: WasmLirLocalId,
    scalar_type: WasmScalarComparisonType,
    context: &LirBodyEmitContext<'_>,
) -> Result<u8, CompilerError> {
    let bits = float_width(scalar_type)?;
    let expected_abi = match bits {
        32 => WasmAbiType::F32,
        64 => WasmAbiType::F64,
        _ => {
            return Err(wasm_generation_error(format!(
                "Wasm scalar comparison cannot use F{bits} in {:?}",
                context.function_id
            )));
        }
    };
    ensure_local_abi(local, expected_abi, context, "float comparison")?;
    function.instruction(&Instruction::LocalGet(local_index(local, context)?));
    Ok(bits)
}

fn emit_float_as_f64(
    function: &mut Function,
    local: WasmLirLocalId,
    scalar_type: WasmScalarComparisonType,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    let bits = emit_float_as_native(function, local, scalar_type, context)?;
    if bits == 32 {
        function.instruction(&Instruction::F64PromoteF32);
    }
    Ok(())
}

fn emit_float_compare(
    function: &mut Function,
    bits: u8,
    op: WasmScalarComparisonOp,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    let instruction = match bits {
        32 => match op {
            WasmScalarComparisonOp::Eq => &Instruction::F32Eq,
            WasmScalarComparisonOp::Ne => &Instruction::F32Ne,
            WasmScalarComparisonOp::Lt => &Instruction::F32Lt,
            WasmScalarComparisonOp::Le => &Instruction::F32Le,
            WasmScalarComparisonOp::Gt => &Instruction::F32Gt,
            WasmScalarComparisonOp::Ge => &Instruction::F32Ge,
        },
        64 => match op {
            WasmScalarComparisonOp::Eq => &Instruction::F64Eq,
            WasmScalarComparisonOp::Ne => &Instruction::F64Ne,
            WasmScalarComparisonOp::Lt => &Instruction::F64Lt,
            WasmScalarComparisonOp::Le => &Instruction::F64Le,
            WasmScalarComparisonOp::Gt => &Instruction::F64Gt,
            WasmScalarComparisonOp::Ge => &Instruction::F64Ge,
        },
        _ => {
            return Err(wasm_generation_error(format!(
                "Wasm scalar comparison cannot use F{bits} in {:?}",
                context.function_id
            )));
        }
    };
    function.instruction(instruction);
    Ok(())
}

fn emit_integer_compare_opcode(
    function: &mut Function,
    abi: WasmAbiType,
    op: WasmScalarComparisonOp,
    unsigned: bool,
) {
    match op {
        WasmScalarComparisonOp::Eq => {
            function.instruction(match abi {
                WasmAbiType::I32 => &Instruction::I32Eq,
                WasmAbiType::I64 => &Instruction::I64Eq,
                _ => unreachable!("integer comparison requires an integer carrier"),
            });
        }
        WasmScalarComparisonOp::Ne => {
            function.instruction(match abi {
                WasmAbiType::I32 => &Instruction::I32Ne,
                WasmAbiType::I64 => &Instruction::I64Ne,
                _ => unreachable!("integer comparison requires an integer carrier"),
            });
        }
        WasmScalarComparisonOp::Lt
        | WasmScalarComparisonOp::Le
        | WasmScalarComparisonOp::Gt
        | WasmScalarComparisonOp::Ge => {
            emit_integer_ordered_opcode(function, abi, ordered_compare_kind(op), unsigned);
        }
    }
}

fn ordered_compare_kind(op: WasmScalarComparisonOp) -> OrderedCompareKind {
    match op {
        WasmScalarComparisonOp::Lt => OrderedCompareKind::Lt,
        WasmScalarComparisonOp::Le => OrderedCompareKind::Le,
        WasmScalarComparisonOp::Gt => OrderedCompareKind::Gt,
        WasmScalarComparisonOp::Ge => OrderedCompareKind::Ge,
        _ => unreachable!("only ordered comparisons have an ordered comparison kind"),
    }
}

fn emit_integer_ordered_opcode(
    function: &mut Function,
    abi: WasmAbiType,
    kind: OrderedCompareKind,
    unsigned: bool,
) {
    let instruction = match abi {
        WasmAbiType::I32 => match (kind, unsigned) {
            (OrderedCompareKind::Lt, false) => &Instruction::I32LtS,
            (OrderedCompareKind::Le, false) => &Instruction::I32LeS,
            (OrderedCompareKind::Gt, false) => &Instruction::I32GtS,
            (OrderedCompareKind::Ge, false) => &Instruction::I32GeS,
            (OrderedCompareKind::Lt, true) => &Instruction::I32LtU,
            (OrderedCompareKind::Le, true) => &Instruction::I32LeU,
            (OrderedCompareKind::Gt, true) => &Instruction::I32GtU,
            (OrderedCompareKind::Ge, true) => &Instruction::I32GeU,
        },
        WasmAbiType::I64 => match (kind, unsigned) {
            (OrderedCompareKind::Lt, false) => &Instruction::I64LtS,
            (OrderedCompareKind::Le, false) => &Instruction::I64LeS,
            (OrderedCompareKind::Gt, false) => &Instruction::I64GtS,
            (OrderedCompareKind::Ge, false) => &Instruction::I64GeS,
            (OrderedCompareKind::Lt, true) => &Instruction::I64LtU,
            (OrderedCompareKind::Le, true) => &Instruction::I64LeU,
            (OrderedCompareKind::Gt, true) => &Instruction::I64GtU,
            (OrderedCompareKind::Ge, true) => &Instruction::I64GeU,
        },
        _ => unreachable!("integer comparison requires an integer carrier"),
    };
    function.instruction(instruction);
}

fn integer_scalar_shape(scalar_type: WasmScalarComparisonType) -> Option<(u8, bool)> {
    match scalar_type {
        WasmScalarComparisonType::SignedInteger(bits) => Some((bits, true)),
        WasmScalarComparisonType::UnsignedInteger(bits) => Some((bits, false)),
        WasmScalarComparisonType::Float(_) => None,
    }
}

fn integer_abi(
    bits: u8,
    scalar_type: WasmScalarComparisonType,
    context: &LirBodyEmitContext<'_>,
) -> Result<WasmAbiType, CompilerError> {
    match bits {
        8 | 16 | 32 => Ok(WasmAbiType::I32),
        64 => Ok(WasmAbiType::I64),
        _ => Err(wasm_generation_error(format!(
            "Wasm scalar comparison received invalid integer type {scalar_type:?} in {:?}",
            context.function_id
        ))),
    }
}

fn float_width(scalar_type: WasmScalarComparisonType) -> Result<u8, CompilerError> {
    match scalar_type {
        WasmScalarComparisonType::Float(bits) => Ok(bits),
        _ => Err(wasm_generation_error(format!(
            "Wasm expected a float scalar type, found {scalar_type:?}"
        ))),
    }
}

pub(super) fn ensure_local_abi(
    local: WasmLirLocalId,
    expected: WasmAbiType,
    context: &LirBodyEmitContext<'_>,
    operation: &str,
) -> Result<(), CompilerError> {
    let actual = local_type(local, context, operation)?;
    if actual != expected {
        return Err(wasm_generation_error(format!(
            "Wasm {operation} expected {expected:?}, found {actual:?} for {local:?} in {:?}",
            context.function_id
        )));
    }
    Ok(())
}

fn invert_ordered_op(op: WasmScalarComparisonOp) -> Result<WasmScalarComparisonOp, CompilerError> {
    match op {
        WasmScalarComparisonOp::Lt => Ok(WasmScalarComparisonOp::Gt),
        WasmScalarComparisonOp::Le => Ok(WasmScalarComparisonOp::Ge),
        WasmScalarComparisonOp::Gt => Ok(WasmScalarComparisonOp::Lt),
        WasmScalarComparisonOp::Ge => Ok(WasmScalarComparisonOp::Le),
        _ => Err(wasm_generation_error(
            "Wasm mixed signed/unsigned comparison requires an ordering operator".to_owned(),
        )),
    }
}

pub(super) fn wasm_generation_error(message: String) -> CompilerError {
    CompilerError::compiler_error(message)
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
}

pub(super) fn local_type(
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

pub(super) fn local_index(
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

pub(super) fn helper_index(
    plan: &WasmEmitPlan,
    helper: WasmRuntimeHelper,
) -> Result<u32, CompilerError> {
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
