//! Trap-mode checked binary-float operation and boundary validation emission.
//!
//! WHAT: emits F32/F64 arithmetic with explicit zero-divisor and post-rounding finite checks;
//!       power and remainder call pure Wasm helpers rather than host imports. Trap-mode
//!       `ValidateFloat` reuses the same native-precision finite check.
//! WHY: native Wasm float instructions produce infinities and NaNs instead of trapping, but Moth
//!      treats every binary-float value as finite and checked failures use the numeric trap path.

use crate::backends::wasm::emit::instructions::{
    LirBodyEmitContext, ensure_local_abi, helper_index, local_index, wasm_generation_error,
};
use crate::backends::wasm::emit::sections::WasmEmitPlan;
use crate::backends::wasm::lir::instructions::WasmNumericOperationOperands;
use crate::backends::wasm::lir::types::{WasmAbiType, WasmLirLocalId};
use crate::backends::wasm::runtime::strings::WasmRuntimeHelper;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use moth_lexical::numeric::precision::BinaryFloatPrecision;
use wasm_encoder::{BlockType, Function, Instruction};

pub(super) fn emit_validate_float(
    function: &mut Function,
    destination: WasmLirLocalId,
    source: WasmLirLocalId,
    precision: BinaryFloatPrecision,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    let abi_type = match precision {
        BinaryFloatPrecision::Binary32 => WasmAbiType::F32,
        BinaryFloatPrecision::Binary64 => WasmAbiType::F64,
        BinaryFloatPrecision::Binary16 => {
            return Err(wasm_generation_error(
                "Wasm Float validation does not support F16".to_owned(),
            ));
        }
    };
    ensure_local_abi(source, abi_type, context, "Float validation source")?;
    ensure_local_abi(
        destination,
        abi_type,
        context,
        "Float validation destination",
    )?;

    function.instruction(&Instruction::LocalGet(local_index(source, context)?));
    emit_finite_float_check(function, destination, precision, context)
}

pub(super) fn emit_checked_float_operation(
    function: &mut Function,
    destination: WasmLirLocalId,
    operator: NumericOperator,
    precision: BinaryFloatPrecision,
    operands: WasmNumericOperationOperands,
    context: &LirBodyEmitContext<'_>,
    plan: &WasmEmitPlan,
) -> Result<(), CompilerError> {
    let abi_type = match precision {
        BinaryFloatPrecision::Binary32 => WasmAbiType::F32,
        BinaryFloatPrecision::Binary64 => WasmAbiType::F64,
        BinaryFloatPrecision::Binary16 => {
            return Err(wasm_generation_error(
                "Wasm checked float operations do not support F16".to_owned(),
            ));
        }
    };
    ensure_local_abi(destination, abi_type, context, "checked float destination")?;

    match (operator, operands) {
        (NumericOperator::Negate, WasmNumericOperationOperands::Unary { operand }) => {
            ensure_local_abi(operand, abi_type, context, "checked float operand")?;
            function.instruction(&Instruction::LocalGet(local_index(operand, context)?));
            function.instruction(match precision {
                BinaryFloatPrecision::Binary32 => &Instruction::F32Neg,
                BinaryFloatPrecision::Binary64 => &Instruction::F64Neg,
                BinaryFloatPrecision::Binary16 => unreachable!("F16 was rejected above"),
            });
        }
        (
            NumericOperator::Add
            | NumericOperator::Subtract
            | NumericOperator::Multiply
            | NumericOperator::Divide,
            WasmNumericOperationOperands::Binary { left, right },
        ) => {
            ensure_local_abi(left, abi_type, context, "checked float left operand")?;
            ensure_local_abi(right, abi_type, context, "checked float right operand")?;

            if operator == NumericOperator::Divide {
                emit_zero_divisor_trap(function, right, precision, context)?;
            }

            function.instruction(&Instruction::LocalGet(local_index(left, context)?));
            function.instruction(&Instruction::LocalGet(local_index(right, context)?));
            function.instruction(native_binary_opcode(operator, precision)?);
        }
        (
            NumericOperator::Remainder | NumericOperator::Power,
            WasmNumericOperationOperands::Binary { left, right },
        ) => {
            ensure_local_abi(left, abi_type, context, "checked float left operand")?;
            ensure_local_abi(right, abi_type, context, "checked float right operand")?;

            if operator == NumericOperator::Remainder {
                emit_zero_divisor_trap(function, right, precision, context)?;
            }

            let helper = match operator {
                NumericOperator::Remainder => WasmRuntimeHelper::FloatRemainder,
                NumericOperator::Power => WasmRuntimeHelper::FloatPower,
                _ => unreachable!("the helper arm only matches remainder or power"),
            };
            emit_float_helper_call(function, helper, left, right, precision, context, plan)?;
        }
        _ => {
            return Err(wasm_generation_error(format!(
                "Wasm checked float operation has an invalid operand shape for {operator:?}"
            )));
        }
    }

    // Native F32 operations and helper-result demotion have already rounded at the semantic
    // precision. Check that rounded value so a finite wider helper result cannot hide F32 overflow.
    emit_finite_float_check(function, destination, precision, context)
}

fn emit_finite_float_check(
    function: &mut Function,
    destination: WasmLirLocalId,
    precision: BinaryFloatPrecision,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    let destination_index = local_index(destination, context)?;
    // Store the original scalar before using abs only for the finite comparison. In particular,
    // the destination retains the source sign bit when the validated value is negative zero.
    function.instruction(&Instruction::LocalTee(destination_index));
    emit_finite_float_predicate(function, precision)?;
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);

    Ok(())
}

/// Replace a native F32/F64 value on the stack with whether it is finite; never traps.
pub(super) fn emit_finite_float_predicate(
    function: &mut Function,
    precision: BinaryFloatPrecision,
) -> Result<(), CompilerError> {
    match precision {
        BinaryFloatPrecision::Binary32 => {
            function.instruction(&Instruction::F32Abs);
            function.instruction(&Instruction::F32Const(f32::MAX.into()));
            function.instruction(&Instruction::F32Le);
        }
        BinaryFloatPrecision::Binary64 => {
            function.instruction(&Instruction::F64Abs);
            function.instruction(&Instruction::F64Const(f64::MAX.into()));
            function.instruction(&Instruction::F64Le);
        }
        BinaryFloatPrecision::Binary16 => {
            return Err(wasm_generation_error(
                "Wasm finite Float checks do not support F16".to_owned(),
            ));
        }
    }

    Ok(())
}

fn native_binary_opcode(
    operator: NumericOperator,
    precision: BinaryFloatPrecision,
) -> Result<&'static Instruction<'static>, CompilerError> {
    let opcode = match (operator, precision) {
        (NumericOperator::Add, BinaryFloatPrecision::Binary32) => &Instruction::F32Add,
        (NumericOperator::Subtract, BinaryFloatPrecision::Binary32) => &Instruction::F32Sub,
        (NumericOperator::Multiply, BinaryFloatPrecision::Binary32) => &Instruction::F32Mul,
        (NumericOperator::Divide, BinaryFloatPrecision::Binary32) => &Instruction::F32Div,
        (NumericOperator::Add, BinaryFloatPrecision::Binary64) => &Instruction::F64Add,
        (NumericOperator::Subtract, BinaryFloatPrecision::Binary64) => &Instruction::F64Sub,
        (NumericOperator::Multiply, BinaryFloatPrecision::Binary64) => &Instruction::F64Mul,
        (NumericOperator::Divide, BinaryFloatPrecision::Binary64) => &Instruction::F64Div,
        (operator, precision) => {
            return Err(wasm_generation_error(format!(
                "Wasm checked float operation cannot emit {operator:?} at {precision:?}"
            )));
        }
    };

    Ok(opcode)
}

fn emit_zero_divisor_trap(
    function: &mut Function,
    divisor: WasmLirLocalId,
    precision: BinaryFloatPrecision,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    function.instruction(&Instruction::LocalGet(local_index(divisor, context)?));
    match precision {
        BinaryFloatPrecision::Binary32 => {
            function.instruction(&Instruction::F32Const(0.0.into()));
            function.instruction(&Instruction::F32Eq);
        }
        BinaryFloatPrecision::Binary64 => {
            function.instruction(&Instruction::F64Const(0.0.into()));
            function.instruction(&Instruction::F64Eq);
        }
        BinaryFloatPrecision::Binary16 => {
            return Err(wasm_generation_error(
                "Wasm checked float operations do not support F16".to_owned(),
            ));
        }
    }
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);
    Ok(())
}

fn emit_float_helper_call(
    function: &mut Function,
    helper: WasmRuntimeHelper,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    precision: BinaryFloatPrecision,
    context: &LirBodyEmitContext<'_>,
    plan: &WasmEmitPlan,
) -> Result<(), CompilerError> {
    function.instruction(&Instruction::LocalGet(local_index(left, context)?));
    if precision == BinaryFloatPrecision::Binary32 {
        function.instruction(&Instruction::F64PromoteF32);
    }

    function.instruction(&Instruction::LocalGet(local_index(right, context)?));
    if precision == BinaryFloatPrecision::Binary32 {
        function.instruction(&Instruction::F64PromoteF32);
    }

    function.instruction(&Instruction::Call(helper_index(plan, helper)?));
    if precision == BinaryFloatPrecision::Binary32 {
        function.instruction(&Instruction::F32DemoteF64);
    }

    Ok(())
}
