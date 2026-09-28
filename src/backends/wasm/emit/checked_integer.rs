//! Native checked integer operation emission for Wasm LIR.
//!
//! WHAT: emits trap-mode integer arithmetic against the operation's signed or unsigned 32/64-bit
//!       semantic domain, with a Wasm `unreachable` on every numeric failure.
//! WHY: Wasm integer instructions wrap and have target-specific traps, so explicit checks must run
//!      before relying on the native result; source operand locals remain untouched.

use crate::backends::wasm::emit::instructions::{
    LirBodyEmitContext, ensure_local_abi, local_index, wasm_generation_error,
};
use crate::backends::wasm::lir::instructions::{
    WasmIntegerOperationKind, WasmIntegerPowerScratch, WasmIntegerScratch,
    WasmNumericOperationOperands,
};
use crate::backends::wasm::lir::types::{WasmAbiType, WasmLirLocalId};
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use wasm_encoder::{BlockType, Function, Instruction};

#[derive(Clone, Copy)]
enum SignedRelation {
    LessThan,
    GreaterThan,
}

pub(super) fn emit_checked_integer_operation(
    function: &mut Function,
    destination: WasmLirLocalId,
    operator: NumericOperator,
    kind: WasmIntegerOperationKind,
    operands: WasmNumericOperationOperands,
    scratch: WasmIntegerScratch,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    let WasmIntegerScratch {
        product: product_scratch,
        power: power_scratch,
    } = scratch;
    let carrier = kind.carrier();
    ensure_local_abi(destination, carrier, context, "checked integer destination")?;

    let (left, right) = match (operator, operands) {
        (NumericOperator::Negate, WasmNumericOperationOperands::Unary { operand }) => {
            ensure_local_abi(operand, carrier, context, "checked integer operand")?;
            (Some(operand), None)
        }
        (
            NumericOperator::Add
            | NumericOperator::Subtract
            | NumericOperator::Multiply
            | NumericOperator::IntegerDivide
            | NumericOperator::Remainder
            | NumericOperator::Power,
            WasmNumericOperationOperands::Binary { left, right },
        ) => {
            ensure_local_abi(left, carrier, context, "checked integer left operand")?;
            ensure_local_abi(right, carrier, context, "checked integer right operand")?;
            (Some(left), Some(right))
        }
        _ => {
            return Err(wasm_generation_error(format!(
                "Wasm checked integer operation has an invalid operand shape for {operator:?}"
            )));
        }
    };

    if left == Some(destination) || right == Some(destination) {
        return Err(wasm_generation_error(
            "Wasm checked integer destination aliases a source operand".to_owned(),
        ));
    }

    let needs_product_scratch =
        is_32_bit(kind) && matches!(operator, NumericOperator::Multiply | NumericOperator::Power);
    let product_scratch = match (needs_product_scratch, product_scratch) {
        (true, Some(scratch)) => {
            ensure_local_abi(
                scratch,
                WasmAbiType::I64,
                context,
                "integer product scratch",
            )?;
            Some(scratch)
        }
        (true, None) => {
            return Err(wasm_generation_error(
                "Wasm 32-bit multiply or power is missing its I64 product scratch".to_owned(),
            ));
        }
        (false, Some(_)) => {
            return Err(wasm_generation_error(
                "Wasm integer operation has unexpected product scratch".to_owned(),
            ));
        }
        (false, None) => None,
    };

    if let Some(product_scratch) = product_scratch
        && (product_scratch == destination
            || left == Some(product_scratch)
            || right == Some(product_scratch))
    {
        return Err(wasm_generation_error(
            "Wasm integer product scratch aliases another operation local".to_owned(),
        ));
    }

    let power_scratch = match (operator, power_scratch) {
        (NumericOperator::Power, Some(scratch)) => Some(scratch),
        (NumericOperator::Power, None) => {
            return Err(wasm_generation_error(
                "Wasm integer power is missing its loop scratch locals".to_owned(),
            ));
        }
        (_, Some(_)) => {
            return Err(wasm_generation_error(
                "Wasm non-power operation has unexpected scratch locals".to_owned(),
            ));
        }
        (_, None) => None,
    };

    if let Some(scratch) = power_scratch {
        ensure_local_abi(scratch.factor, carrier, context, "integer power factor")?;
        ensure_local_abi(scratch.exponent, carrier, context, "integer power exponent")?;
        let (left_operand, right_operand) = match (left, right) {
            (Some(left_operand), Some(right_operand)) => (left_operand, right_operand),
            _ => {
                return Err(wasm_generation_error(
                    "Wasm integer power has an invalid operand shape".to_owned(),
                ));
            }
        };
        if scratch.factor == scratch.exponent
            || scratch.factor == destination
            || scratch.exponent == destination
            || scratch.factor == left_operand
            || scratch.factor == right_operand
            || scratch.exponent == left_operand
            || scratch.exponent == right_operand
            || product_scratch == Some(scratch.factor)
            || product_scratch == Some(scratch.exponent)
        {
            return Err(wasm_generation_error(
                "Wasm integer power scratch locals alias another operation local".to_owned(),
            ));
        }
    }

    match (operator, left, right, power_scratch) {
        (NumericOperator::Negate, Some(operand), None, None) => {
            emit_checked_negation(function, destination, operand, kind, context)
        }
        (NumericOperator::Add, Some(left), Some(right), None) => {
            emit_checked_add(function, destination, left, right, kind, context)
        }
        (NumericOperator::Subtract, Some(left), Some(right), None) => {
            emit_checked_subtract(function, destination, left, right, kind, context)
        }
        (NumericOperator::Multiply, Some(left), Some(right), None) => emit_checked_multiply_into(
            function,
            destination,
            left,
            right,
            kind,
            product_scratch,
            context,
        ),
        (NumericOperator::IntegerDivide, Some(left), Some(right), None) => {
            emit_checked_division(function, destination, left, right, kind, context)
        }
        (NumericOperator::Remainder, Some(left), Some(right), None) => {
            emit_checked_remainder(function, destination, left, right, kind, context)
        }
        (NumericOperator::Power, Some(left), Some(right), Some(_)) => {
            emit_checked_power(function, destination, left, right, kind, scratch, context)
        }
        _ => Err(wasm_generation_error(format!(
            "Wasm checked integer operation has inconsistent scratch locals for {operator:?}"
        ))),
    }
}

fn emit_checked_add(
    function: &mut Function,
    destination: WasmLirLocalId,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    emit_native_integer_binary(function, left, right, kind, NumericOperator::Add, context)?;
    function.instruction(&Instruction::LocalSet(local_index(destination, context)?));

    match kind {
        WasmIntegerOperationKind::Signed32 => {
            function.instruction(&Instruction::LocalGet(local_index(left, context)?));
            function.instruction(&Instruction::LocalGet(local_index(destination, context)?));
            function.instruction(&Instruction::I32Xor);
            function.instruction(&Instruction::LocalGet(local_index(right, context)?));
            function.instruction(&Instruction::LocalGet(local_index(destination, context)?));
            function.instruction(&Instruction::I32Xor);
            function.instruction(&Instruction::I32And);
            function.instruction(&Instruction::I32Const(0));
            function.instruction(&Instruction::I32LtS);
        }
        WasmIntegerOperationKind::Signed64 => {
            function.instruction(&Instruction::LocalGet(local_index(left, context)?));
            function.instruction(&Instruction::LocalGet(local_index(destination, context)?));
            function.instruction(&Instruction::I64Xor);
            function.instruction(&Instruction::LocalGet(local_index(right, context)?));
            function.instruction(&Instruction::LocalGet(local_index(destination, context)?));
            function.instruction(&Instruction::I64Xor);
            function.instruction(&Instruction::I64And);
            function.instruction(&Instruction::I64Const(0));
            function.instruction(&Instruction::I64LtS);
        }
        WasmIntegerOperationKind::Unsigned32 => {
            function.instruction(&Instruction::LocalGet(local_index(destination, context)?));
            function.instruction(&Instruction::LocalGet(local_index(left, context)?));
            function.instruction(&Instruction::I32LtU);
        }
        WasmIntegerOperationKind::Unsigned64 => {
            function.instruction(&Instruction::LocalGet(local_index(destination, context)?));
            function.instruction(&Instruction::LocalGet(local_index(left, context)?));
            function.instruction(&Instruction::I64LtU);
        }
    }
    emit_unreachable_if_true(function);
    Ok(())
}

fn emit_checked_subtract(
    function: &mut Function,
    destination: WasmLirLocalId,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    emit_native_integer_binary(
        function,
        left,
        right,
        kind,
        NumericOperator::Subtract,
        context,
    )?;
    function.instruction(&Instruction::LocalSet(local_index(destination, context)?));

    match kind {
        WasmIntegerOperationKind::Signed32 => {
            function.instruction(&Instruction::LocalGet(local_index(left, context)?));
            function.instruction(&Instruction::LocalGet(local_index(right, context)?));
            function.instruction(&Instruction::I32Xor);
            function.instruction(&Instruction::LocalGet(local_index(left, context)?));
            function.instruction(&Instruction::LocalGet(local_index(destination, context)?));
            function.instruction(&Instruction::I32Xor);
            function.instruction(&Instruction::I32And);
            function.instruction(&Instruction::I32Const(0));
            function.instruction(&Instruction::I32LtS);
        }
        WasmIntegerOperationKind::Signed64 => {
            function.instruction(&Instruction::LocalGet(local_index(left, context)?));
            function.instruction(&Instruction::LocalGet(local_index(right, context)?));
            function.instruction(&Instruction::I64Xor);
            function.instruction(&Instruction::LocalGet(local_index(left, context)?));
            function.instruction(&Instruction::LocalGet(local_index(destination, context)?));
            function.instruction(&Instruction::I64Xor);
            function.instruction(&Instruction::I64And);
            function.instruction(&Instruction::I64Const(0));
            function.instruction(&Instruction::I64LtS);
        }
        WasmIntegerOperationKind::Unsigned32 | WasmIntegerOperationKind::Unsigned64 => {
            emit_unsigned_less_than(function, left, right, kind, context)?;
        }
    }
    emit_unreachable_if_true(function);
    Ok(())
}

fn emit_checked_negation(
    function: &mut Function,
    destination: WasmLirLocalId,
    operand: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    let Some((minimum, _)) = signed_bounds(kind) else {
        return Err(wasm_generation_error(
            "Wasm checked negation requires a signed integer domain".to_owned(),
        ));
    };

    emit_local_equals_constant(function, operand, kind, minimum, context)?;
    emit_unreachable_if_true(function);
    emit_integer_constant(function, kind, 0);
    function.instruction(&Instruction::LocalGet(local_index(operand, context)?));
    function.instruction(integer_opcode(kind, NumericOperator::Subtract)?);
    function.instruction(&Instruction::LocalSet(local_index(destination, context)?));
    Ok(())
}

fn emit_checked_division(
    function: &mut Function,
    destination: WasmLirLocalId,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    emit_local_is_zero(function, right, kind, context)?;
    emit_unreachable_if_true(function);

    if let Some((minimum, _)) = signed_bounds(kind) {
        emit_local_equals_constant(function, left, kind, minimum, context)?;
        emit_local_equals_constant(function, right, kind, -1, context)?;
        function.instruction(&Instruction::I32And);
        emit_unreachable_if_true(function);
    }

    emit_native_integer_binary(
        function,
        left,
        right,
        kind,
        NumericOperator::IntegerDivide,
        context,
    )?;
    function.instruction(&Instruction::LocalSet(local_index(destination, context)?));
    Ok(())
}

fn emit_checked_remainder(
    function: &mut Function,
    destination: WasmLirLocalId,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    emit_local_is_zero(function, right, kind, context)?;
    emit_unreachable_if_true(function);
    emit_native_integer_binary(
        function,
        left,
        right,
        kind,
        NumericOperator::Remainder,
        context,
    )?;
    function.instruction(&Instruction::LocalSet(local_index(destination, context)?));
    Ok(())
}

fn emit_checked_power(
    function: &mut Function,
    destination: WasmLirLocalId,
    base: WasmLirLocalId,
    source_exponent: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    scratch: WasmIntegerScratch,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    let product_scratch = scratch.product;
    let Some(WasmIntegerPowerScratch { factor, exponent }) = scratch.power else {
        return Err(wasm_generation_error(
            "Wasm integer power is missing its loop scratch locals".to_owned(),
        ));
    };

    if is_signed(kind) {
        emit_signed_local_relation(
            function,
            source_exponent,
            kind,
            SignedRelation::LessThan,
            context,
        )?;
        emit_unreachable_if_true(function);
    }

    function.instruction(&Instruction::LocalGet(local_index(base, context)?));
    function.instruction(&Instruction::LocalSet(local_index(factor, context)?));
    function.instruction(&Instruction::LocalGet(local_index(
        source_exponent,
        context,
    )?));
    function.instruction(&Instruction::LocalSet(local_index(exponent, context)?));
    // A trap aborts before the caller commits this result, so it can hold the accumulator.
    emit_integer_constant(function, kind, 1);
    function.instruction(&Instruction::LocalSet(local_index(destination, context)?));

    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    emit_local_is_zero(function, exponent, kind, context)?;
    function.instruction(&Instruction::BrIf(1));

    emit_local_is_odd(function, exponent, kind, context)?;
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_checked_multiply_into(
        function,
        destination,
        destination,
        factor,
        kind,
        product_scratch,
        context,
    )?;
    function.instruction(&Instruction::End);

    // An unused final factor square must not introduce an overflow past the result.
    emit_shift_exponent_right(function, exponent, kind, context)?;
    emit_local_is_zero(function, exponent, kind, context)?;
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_checked_multiply_into(
        function,
        factor,
        factor,
        factor,
        kind,
        product_scratch,
        context,
    )?;
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::Br(0));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
    Ok(())
}

fn emit_checked_multiply_into(
    function: &mut Function,
    destination: WasmLirLocalId,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    product_scratch: Option<WasmLirLocalId>,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    match kind {
        WasmIntegerOperationKind::Signed32 | WasmIntegerOperationKind::Unsigned32 => {
            let product_scratch = product_scratch.ok_or_else(|| {
                wasm_generation_error(
                    "Wasm 32-bit multiply is missing its I64 product scratch".to_owned(),
                )
            })?;
            let extend = match kind {
                WasmIntegerOperationKind::Signed32 => &Instruction::I64ExtendI32S,
                WasmIntegerOperationKind::Unsigned32 => &Instruction::I64ExtendI32U,
                _ => unreachable!(),
            };
            function.instruction(&Instruction::LocalGet(local_index(left, context)?));
            function.instruction(extend);
            function.instruction(&Instruction::LocalGet(local_index(right, context)?));
            function.instruction(extend);
            function.instruction(&Instruction::I64Mul);
            function.instruction(&Instruction::LocalSet(local_index(
                product_scratch,
                context,
            )?));

            match kind {
                WasmIntegerOperationKind::Signed32 => {
                    function.instruction(&Instruction::LocalGet(local_index(
                        product_scratch,
                        context,
                    )?));
                    function.instruction(&Instruction::I64Const(i64::from(i32::MIN)));
                    function.instruction(&Instruction::I64LtS);
                    emit_unreachable_if_true(function);
                    function.instruction(&Instruction::LocalGet(local_index(
                        product_scratch,
                        context,
                    )?));
                    function.instruction(&Instruction::I64Const(i64::from(i32::MAX)));
                    function.instruction(&Instruction::I64GtS);
                    emit_unreachable_if_true(function);
                }
                WasmIntegerOperationKind::Unsigned32 => {
                    function.instruction(&Instruction::LocalGet(local_index(
                        product_scratch,
                        context,
                    )?));
                    function.instruction(&Instruction::I64Const(i64::from(u32::MAX)));
                    function.instruction(&Instruction::I64GtU);
                    emit_unreachable_if_true(function);
                }
                _ => unreachable!(),
            }

            function.instruction(&Instruction::LocalGet(local_index(
                product_scratch,
                context,
            )?));
            function.instruction(&Instruction::I32WrapI64);
            function.instruction(&Instruction::LocalSet(local_index(destination, context)?));
            Ok(())
        }
        WasmIntegerOperationKind::Signed64 | WasmIntegerOperationKind::Unsigned64 => {
            if product_scratch.is_some() {
                return Err(wasm_generation_error(
                    "Wasm 64-bit multiply has unexpected product scratch".to_owned(),
                ));
            }
            emit_64_bit_multiply_overflow_check(function, left, right, kind, context)?;
            emit_native_integer_binary(
                function,
                left,
                right,
                kind,
                NumericOperator::Multiply,
                context,
            )?;
            function.instruction(&Instruction::LocalSet(local_index(destination, context)?));
            Ok(())
        }
    }
}

fn emit_64_bit_multiply_overflow_check(
    function: &mut Function,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    match kind {
        WasmIntegerOperationKind::Signed64 => {
            emit_signed_multiply_overflow_check(function, left, right, context)
        }
        WasmIntegerOperationKind::Unsigned64 => {
            emit_local_is_zero(function, left, kind, context)?;
            function.instruction(&Instruction::I32Eqz);
            function.instruction(&Instruction::If(BlockType::Empty));
            function.instruction(&Instruction::LocalGet(local_index(right, context)?));
            function.instruction(&Instruction::I64Const(-1));
            function.instruction(&Instruction::LocalGet(local_index(left, context)?));
            function.instruction(&Instruction::I64DivU);
            function.instruction(&Instruction::I64GtU);
            emit_unreachable_if_true(function);
            function.instruction(&Instruction::End);
            Ok(())
        }
        _ => Err(wasm_generation_error(
            "Wasm widened multiply check requires a 64-bit integer domain".to_owned(),
        )),
    }
}

fn emit_signed_multiply_overflow_check(
    function: &mut Function,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    let kind = WasmIntegerOperationKind::Signed64;
    let minimum = i64::MIN;
    let maximum = i64::MAX;

    // These sign branches make every quotient denominator nonzero and avoid MIN / -1.
    emit_signed_local_relation(function, right, kind, SignedRelation::GreaterThan, context)?;
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_signed_local_relation(function, left, kind, SignedRelation::GreaterThan, context)?;
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_signed_bound_failure(
        function,
        left,
        maximum,
        right,
        SignedRelation::GreaterThan,
        context,
    )?;
    function.instruction(&Instruction::Else);
    emit_signed_local_relation(function, left, kind, SignedRelation::LessThan, context)?;
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_signed_bound_failure(
        function,
        left,
        minimum,
        right,
        SignedRelation::LessThan,
        context,
    )?;
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::Else);
    emit_signed_local_relation(function, right, kind, SignedRelation::LessThan, context)?;
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_signed_local_relation(function, left, kind, SignedRelation::GreaterThan, context)?;
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_signed_bound_failure(
        function,
        right,
        minimum,
        left,
        SignedRelation::LessThan,
        context,
    )?;
    function.instruction(&Instruction::Else);
    emit_signed_local_relation(function, left, kind, SignedRelation::LessThan, context)?;
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_signed_bound_failure(
        function,
        right,
        maximum,
        left,
        SignedRelation::LessThan,
        context,
    )?;
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
    Ok(())
}

fn emit_signed_bound_failure(
    function: &mut Function,
    compared: WasmLirLocalId,
    numerator: i64,
    denominator: WasmLirLocalId,
    relation: SignedRelation,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    function.instruction(&Instruction::LocalGet(local_index(compared, context)?));
    function.instruction(&Instruction::I64Const(numerator));
    function.instruction(&Instruction::LocalGet(local_index(denominator, context)?));
    function.instruction(&Instruction::I64DivS);
    emit_signed_relation_opcode(function, WasmIntegerOperationKind::Signed64, relation);
    emit_unreachable_if_true(function);
    Ok(())
}

fn emit_signed_local_relation(
    function: &mut Function,
    local: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    relation: SignedRelation,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    function.instruction(&Instruction::LocalGet(local_index(local, context)?));
    emit_integer_constant(function, kind, 0);
    emit_signed_relation_opcode(function, kind, relation);
    Ok(())
}

fn emit_unsigned_less_than(
    function: &mut Function,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    function.instruction(&Instruction::LocalGet(local_index(left, context)?));
    function.instruction(&Instruction::LocalGet(local_index(right, context)?));
    function.instruction(match kind {
        WasmIntegerOperationKind::Unsigned32 => &Instruction::I32LtU,
        WasmIntegerOperationKind::Unsigned64 => &Instruction::I64LtU,
        _ => unreachable!(),
    });
    Ok(())
}

fn emit_signed_relation_opcode(
    function: &mut Function,
    kind: WasmIntegerOperationKind,
    relation: SignedRelation,
) {
    function.instruction(match (kind.carrier(), relation) {
        (WasmAbiType::I32, SignedRelation::LessThan) => &Instruction::I32LtS,
        (WasmAbiType::I32, SignedRelation::GreaterThan) => &Instruction::I32GtS,
        (WasmAbiType::I64, SignedRelation::LessThan) => &Instruction::I64LtS,
        (WasmAbiType::I64, SignedRelation::GreaterThan) => &Instruction::I64GtS,
        _ => unreachable!(),
    });
}

fn emit_local_equals_constant(
    function: &mut Function,
    local: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    value: i64,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    function.instruction(&Instruction::LocalGet(local_index(local, context)?));
    emit_integer_constant(function, kind, value);
    function.instruction(match kind.carrier() {
        WasmAbiType::I32 => &Instruction::I32Eq,
        WasmAbiType::I64 => &Instruction::I64Eq,
        _ => unreachable!(),
    });
    Ok(())
}

fn emit_local_is_zero(
    function: &mut Function,
    local: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    function.instruction(&Instruction::LocalGet(local_index(local, context)?));
    function.instruction(match kind.carrier() {
        WasmAbiType::I32 => &Instruction::I32Eqz,
        WasmAbiType::I64 => &Instruction::I64Eqz,
        _ => unreachable!(),
    });
    Ok(())
}

fn emit_local_is_odd(
    function: &mut Function,
    local: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    function.instruction(&Instruction::LocalGet(local_index(local, context)?));
    emit_integer_constant(function, kind, 1);
    function.instruction(match kind.carrier() {
        WasmAbiType::I32 => &Instruction::I32And,
        WasmAbiType::I64 => &Instruction::I64And,
        _ => unreachable!(),
    });
    emit_integer_constant(function, kind, 0);
    function.instruction(match kind.carrier() {
        WasmAbiType::I32 => &Instruction::I32Ne,
        WasmAbiType::I64 => &Instruction::I64Ne,
        _ => unreachable!(),
    });
    Ok(())
}

fn emit_shift_exponent_right(
    function: &mut Function,
    exponent: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    function.instruction(&Instruction::LocalGet(local_index(exponent, context)?));
    emit_integer_constant(function, kind, 1);
    function.instruction(match kind.carrier() {
        WasmAbiType::I32 => &Instruction::I32ShrU,
        WasmAbiType::I64 => &Instruction::I64ShrU,
        _ => unreachable!(),
    });
    function.instruction(&Instruction::LocalSet(local_index(exponent, context)?));
    Ok(())
}

fn emit_native_integer_binary(
    function: &mut Function,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    operator: NumericOperator,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), CompilerError> {
    function.instruction(&Instruction::LocalGet(local_index(left, context)?));
    function.instruction(&Instruction::LocalGet(local_index(right, context)?));
    function.instruction(integer_opcode(kind, operator)?);
    Ok(())
}

fn integer_opcode(
    kind: WasmIntegerOperationKind,
    operator: NumericOperator,
) -> Result<&'static Instruction<'static>, CompilerError> {
    let opcode = match (kind, operator) {
        (WasmIntegerOperationKind::Signed32, NumericOperator::Add) => &Instruction::I32Add,
        (WasmIntegerOperationKind::Unsigned32, NumericOperator::Add) => &Instruction::I32Add,
        (WasmIntegerOperationKind::Signed32, NumericOperator::Subtract) => &Instruction::I32Sub,
        (WasmIntegerOperationKind::Unsigned32, NumericOperator::Subtract) => &Instruction::I32Sub,
        (WasmIntegerOperationKind::Signed32, NumericOperator::Multiply) => &Instruction::I32Mul,
        (WasmIntegerOperationKind::Unsigned32, NumericOperator::Multiply) => &Instruction::I32Mul,
        (WasmIntegerOperationKind::Signed32, NumericOperator::IntegerDivide) => {
            &Instruction::I32DivS
        }
        (WasmIntegerOperationKind::Unsigned32, NumericOperator::IntegerDivide) => {
            &Instruction::I32DivU
        }
        (WasmIntegerOperationKind::Signed32, NumericOperator::Remainder) => &Instruction::I32RemS,
        (WasmIntegerOperationKind::Unsigned32, NumericOperator::Remainder) => &Instruction::I32RemU,
        (WasmIntegerOperationKind::Signed64, NumericOperator::Add) => &Instruction::I64Add,
        (WasmIntegerOperationKind::Unsigned64, NumericOperator::Add) => &Instruction::I64Add,
        (WasmIntegerOperationKind::Signed64, NumericOperator::Subtract) => &Instruction::I64Sub,
        (WasmIntegerOperationKind::Unsigned64, NumericOperator::Subtract) => &Instruction::I64Sub,
        (WasmIntegerOperationKind::Signed64, NumericOperator::Multiply) => &Instruction::I64Mul,
        (WasmIntegerOperationKind::Unsigned64, NumericOperator::Multiply) => &Instruction::I64Mul,
        (WasmIntegerOperationKind::Signed64, NumericOperator::IntegerDivide) => {
            &Instruction::I64DivS
        }
        (WasmIntegerOperationKind::Unsigned64, NumericOperator::IntegerDivide) => {
            &Instruction::I64DivU
        }
        (WasmIntegerOperationKind::Signed64, NumericOperator::Remainder) => &Instruction::I64RemS,
        (WasmIntegerOperationKind::Unsigned64, NumericOperator::Remainder) => &Instruction::I64RemU,
        (_, operator) => {
            return Err(wasm_generation_error(format!(
                "Wasm checked integer opcode does not support {operator:?}"
            )));
        }
    };
    Ok(opcode)
}

fn emit_integer_constant(function: &mut Function, kind: WasmIntegerOperationKind, value: i64) {
    match kind.carrier() {
        WasmAbiType::I32 => function.instruction(&Instruction::I32Const(value as i32)),
        WasmAbiType::I64 => function.instruction(&Instruction::I64Const(value)),
        WasmAbiType::F32 | WasmAbiType::F64 | WasmAbiType::Handle | WasmAbiType::Void => {
            unreachable!()
        }
    };
}

fn emit_unreachable_if_true(function: &mut Function) {
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);
}

fn signed_bounds(kind: WasmIntegerOperationKind) -> Option<(i64, i64)> {
    match kind {
        WasmIntegerOperationKind::Signed32 => Some((i64::from(i32::MIN), i64::from(i32::MAX))),
        WasmIntegerOperationKind::Signed64 => Some((i64::MIN, i64::MAX)),
        WasmIntegerOperationKind::Unsigned32 | WasmIntegerOperationKind::Unsigned64 => None,
    }
}

fn is_signed(kind: WasmIntegerOperationKind) -> bool {
    matches!(
        kind,
        WasmIntegerOperationKind::Signed32 | WasmIntegerOperationKind::Signed64
    )
}

fn is_32_bit(kind: WasmIntegerOperationKind) -> bool {
    matches!(
        kind,
        WasmIntegerOperationKind::Signed32 | WasmIntegerOperationKind::Unsigned32
    )
}
