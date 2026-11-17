//! Native integer operation emission for Wasm LIR.
//!
//! WHAT: emits the runtime-checked, proven-safe and `$fast_math` integer arithmetic families
//!       against the operation's signed or unsigned 32/64-bit semantic domain, with a Wasm
//!       `unreachable` for every fault the selected family reports.
//! WHY: Wasm integer instructions wrap and have target-specific traps, so runtime-checked
//!      operations must verify every failure before relying on the native result; source operand
//!      locals remain untouched. Proven-safe statements skip only redundant predicates because
//!      the analysis table already excluded those exact failures, and fast statements wrap by
//!      language rule without any overflow predicate. Both no-check families may write an
//!      aliasing destination directly because operands are read before the result `local.set`.

use crate::backends::wasm::emit::instructions::{
    LirBodyEmitContext, ensure_local_abi, local_index,
};
use crate::backends::wasm::lir::instructions::{
    IntegerCheckMode, WasmCheckedIntegerOperation, WasmFastIntegerOperation,
    WasmIntegerOperationKind, WasmIntegerPowerScratch, WasmIntegerScratch,
    WasmNumericOperationOperands,
};
use crate::backends::wasm::lir::types::{WasmAbiType, WasmLirLocalId};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use wasm_encoder::{BlockType, Function, Instruction};

#[derive(Clone, Copy)]
enum SignedRelation {
    LessThan,
    GreaterThan,
}

/// Resolves one integer operation's operand shape for the checked and fast emitters.
///
/// WHAT: accepts unary `Negate` and the six binary operators, verifying every operand local's
///       native carrier.
/// WHY: both integer families read the same HIR-derived operand shape, so one resolver keeps
///      them from accepting different operator sets.
fn resolve_integer_operands(
    operator: NumericOperator,
    operands: WasmNumericOperationOperands,
    carrier: WasmAbiType,
    context: &LirBodyEmitContext<'_>,
    family: &str,
) -> Result<(Option<WasmLirLocalId>, Option<WasmLirLocalId>), InfrastructureFailure> {
    match (operator, operands) {
        (NumericOperator::Negate, WasmNumericOperationOperands::Unary { operand }) => {
            ensure_local_abi(
                operand,
                carrier,
                context,
                &format!("{family} integer operand"),
            )?;
            Ok((Some(operand), None))
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
            ensure_local_abi(
                left,
                carrier,
                context,
                &format!("{family} integer left operand"),
            )?;
            ensure_local_abi(
                right,
                carrier,
                context,
                &format!("{family} integer right operand"),
            )?;
            Ok((Some(left), Some(right)))
        }
        _ => Err(InfrastructureFailure::wasm_generation(format!(
            "Wasm {family} integer operation has an invalid operand shape for {operator:?}"
        ))),
    }
}

/// Verifies that power's loop scratch locals stay distinct from every other operation local.
///
/// WHY: checked and fast power share the square-and-multiply loop, so they share its aliasing
///      contract; only the checked family has an extra widened product scratch to exclude.
fn ensure_power_scratch_is_distinct(
    scratch: WasmIntegerPowerScratch,
    carrier: WasmAbiType,
    destination: WasmLirLocalId,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    product_scratch: Option<WasmLirLocalId>,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), InfrastructureFailure> {
    ensure_local_abi(scratch.factor, carrier, context, "integer power factor")?;
    ensure_local_abi(scratch.exponent, carrier, context, "integer power exponent")?;
    if scratch.factor == scratch.exponent
        || scratch.factor == destination
        || scratch.exponent == destination
        || scratch.factor == left
        || scratch.factor == right
        || scratch.exponent == left
        || scratch.exponent == right
        || product_scratch == Some(scratch.factor)
        || product_scratch == Some(scratch.exponent)
    {
        return Err(InfrastructureFailure::wasm_generation(
            "Wasm integer power scratch locals alias another operation local".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn emit_checked_integer_operation(
    function: &mut Function,
    operation: &WasmCheckedIntegerOperation,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), InfrastructureFailure> {
    let WasmCheckedIntegerOperation {
        dst: destination,
        operator,
        kind,
        check_mode,
        operands,
        scratch,
    } = operation;
    let WasmIntegerScratch {
        product: product_scratch,
        power: power_scratch,
    } = scratch;
    let carrier = kind.carrier();
    ensure_local_abi(
        *destination,
        carrier,
        context,
        "checked integer destination",
    )?;

    let (left, right) =
        resolve_integer_operands(*operator, *operands, carrier, context, "checked")?;

    if left == Some(*destination) || right == Some(*destination) {
        // Runtime lowering needs the pre-operation operand values for its predicates; proven-safe
        // native arithmetic reads its operands before the destination `local.set`, so a proven
        // statement keeps its alias-safe write directly.
        if *check_mode == IntegerCheckMode::Runtime {
            return Err(InfrastructureFailure::wasm_generation(
                "Wasm checked integer destination aliases a source operand".to_owned(),
            ));
        }
        if *operator == NumericOperator::Power {
            return Err(InfrastructureFailure::wasm_generation(
                "Wasm proven-safe integer operation cannot target Power".to_owned(),
            ));
        }
    }

    // Scratch remains allocation policy: a proven-safe operation omits check-only locals, while
    // a runtime operation validates that it received every scratch its predicates need.
    let needs_product_scratch = *check_mode == IntegerCheckMode::Runtime
        && is_32_bit(*kind)
        && matches!(operator, NumericOperator::Multiply | NumericOperator::Power);
    let product_scratch = match (needs_product_scratch, product_scratch) {
        (true, Some(scratch)) => {
            ensure_local_abi(
                *scratch,
                WasmAbiType::I64,
                context,
                "integer product scratch",
            )?;
            Some(*scratch)
        }
        (true, None) => {
            return Err(InfrastructureFailure::wasm_generation(
                "Wasm 32-bit multiply or power is missing its I64 product scratch".to_owned(),
            ));
        }
        (false, Some(_)) => {
            return Err(InfrastructureFailure::wasm_generation(
                "Wasm integer operation has unexpected product scratch".to_owned(),
            ));
        }
        (false, None) => None,
    };

    if let Some(product_scratch) = product_scratch
        && (product_scratch == *destination
            || left == Some(product_scratch)
            || right == Some(product_scratch))
    {
        return Err(InfrastructureFailure::wasm_generation(
            "Wasm integer product scratch aliases another operation local".to_owned(),
        ));
    }

    let power_scratch = match (*operator, power_scratch) {
        (NumericOperator::Power, Some(scratch)) => Some(*scratch),
        (NumericOperator::Power, None) => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm integer power is missing its loop scratch locals"
        ),
        (_, Some(_)) => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm non-power operation has unexpected scratch locals"
        ),
        (_, None) => None,
    };

    if let Some(scratch) = power_scratch {
        if *check_mode != IntegerCheckMode::Runtime {
            return Err(InfrastructureFailure::wasm_generation(
                "Wasm proven-safe integer operation cannot be Power".to_owned(),
            ));
        }
        let (Some(left_operand), Some(right_operand)) = (left, right) else {
            return Err(InfrastructureFailure::wasm_generation(
                "Wasm integer power has an invalid operand shape".to_owned(),
            ));
        };
        ensure_power_scratch_is_distinct(
            scratch,
            carrier,
            *destination,
            left_operand,
            right_operand,
            product_scratch,
            context,
        )?;
    }

    if *check_mode == IntegerCheckMode::ProvenSafe {
        // Statement-specific proofs cover Add, Subtract, Multiply, Negate, IntegerDivide and
        // Remainder only; Power never reaches this mode. Each native opcode reads its operands
        // before the destination `local.set`, so aliasing destinations stay semantics-safe;
        // proven div/rem prove the divisor non-zero and, for division, exclude the signed
        // minimum/-1 overflow, and proven negation proves the operand is not the signed minimum.
        let (left, right) = match (*operator, left, right) {
            (NumericOperator::Negate, Some(operand), None) => (Some(operand), None),
            (
                NumericOperator::Add
                | NumericOperator::Subtract
                | NumericOperator::Multiply
                | NumericOperator::IntegerDivide
                | NumericOperator::Remainder,
                Some(left),
                Some(right),
            ) => (Some(left), Some(right)),
            _ => {
                return Err(InfrastructureFailure::wasm_generation(format!(
                    "Wasm proven-safe integer operation has an unsupported shape for {operator:?}"
                )));
            }
        };
        return match (*operator, left, right) {
            (NumericOperator::Negate, Some(operand), None) => {
                emit_wrapping_negation(function, *destination, operand, *kind, context)
            }
            (
                NumericOperator::Add
                | NumericOperator::Subtract
                | NumericOperator::Multiply
                | NumericOperator::IntegerDivide
                | NumericOperator::Remainder,
                Some(left),
                Some(right),
            ) => {
                emit_native_integer_binary(function, left, right, *kind, *operator, context)?;
                function.instruction(&Instruction::LocalSet(local_index(*destination, context)?));
                Ok(())
            }
            _ => crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "proven-safe integer emit already validated Negate as unary and Add/Subtract/Multiply/IntegerDivide/Remainder as binary"
            ),
        };
    }

    match (*operator, left, right, power_scratch) {
        (NumericOperator::Negate, Some(operand), None, None) => {
            emit_checked_negation(function, *destination, operand, *kind, context)
        }
        (NumericOperator::Add, Some(left), Some(right), None) => {
            emit_checked_add(function, *destination, left, right, *kind, context)
        }
        (NumericOperator::Subtract, Some(left), Some(right), None) => {
            emit_checked_subtract(function, *destination, left, right, *kind, context)
        }
        (NumericOperator::Multiply, Some(left), Some(right), None) => emit_checked_multiply_into(
            function,
            *destination,
            left,
            right,
            *kind,
            product_scratch,
            context,
        ),
        (NumericOperator::IntegerDivide, Some(left), Some(right), None) => {
            emit_guarded_division(function, *destination, left, right, *kind, context)
        }
        (NumericOperator::Remainder, Some(left), Some(right), None) => {
            emit_guarded_remainder(function, *destination, left, right, *kind, context)
        }
        (NumericOperator::Power, Some(left), Some(right), Some(power_scratch)) => {
            emit_integer_power_loop(
                function,
                IntegerPowerOperation {
                    destination: *destination,
                    base: left,
                    source_exponent: right,
                    kind: *kind,
                    scratch: power_scratch,
                    product: IntegerPowerProduct::Checked { product_scratch },
                },
                context,
            )
        }
        _ => Err(InfrastructureFailure::wasm_generation(format!(
            "Wasm checked integer operation has inconsistent scratch locals for {operator:?}"
        ))),
    }
}

/// Emits one `$fast_math` integer operation.
///
/// WHAT: native modular `+`, `-`, `*` and unary `-`; truncating `//`/`%` guarded only by their
///       fast trap causes; and square-and-multiply power in the semantic width.
/// WHY: fast integer arithmetic is neither runtime-checked nor analysis-proven: it must wrap by
///      language rule, so emitting the ordinary overflow predicates would reject valid results.
///      Traps use the same `unreachable` transport as the ordinary trap path, but only for the
///      causes `fast_numeric_trap_codes` names.
pub(super) fn emit_fast_integer_operation(
    function: &mut Function,
    operation: &WasmFastIntegerOperation,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), InfrastructureFailure> {
    let WasmFastIntegerOperation {
        dst: destination,
        operator,
        kind,
        operands,
        power,
    } = *operation;
    let carrier = kind.carrier();
    ensure_local_abi(destination, carrier, context, "fast integer destination")?;

    let (left, right) = resolve_integer_operands(operator, operands, carrier, context, "fast")?;

    let power_scratch = match (operator, power) {
        (NumericOperator::Power, Some(scratch)) => {
            let (Some(left_operand), Some(right_operand)) = (left, right) else {
                return Err(InfrastructureFailure::wasm_generation(
                    "Wasm fast integer power has an invalid operand shape".to_owned(),
                ));
            };
            ensure_power_scratch_is_distinct(
                scratch,
                carrier,
                destination,
                left_operand,
                right_operand,
                None,
                context,
            )?;
            Some(scratch)
        }
        (NumericOperator::Power, None) => {
            return Err(InfrastructureFailure::wasm_generation(
                "Wasm fast integer power is missing its loop scratch locals".to_owned(),
            ));
        }
        (_, Some(_)) => {
            return Err(InfrastructureFailure::wasm_generation(
                "Wasm fast non-power integer operation has unexpected scratch locals".to_owned(),
            ));
        }
        (_, None) => None,
    };

    match (operator, left, right, power_scratch) {
        (NumericOperator::Negate, Some(operand), None, None) => {
            emit_wrapping_negation(function, destination, operand, kind, context)
        }
        (NumericOperator::Add, Some(left), Some(right), None) => emit_fast_native_binary(
            function,
            destination,
            left,
            right,
            kind,
            NumericOperator::Add,
            context,
        ),
        (NumericOperator::Subtract, Some(left), Some(right), None) => emit_fast_native_binary(
            function,
            destination,
            left,
            right,
            kind,
            NumericOperator::Subtract,
            context,
        ),
        (NumericOperator::Multiply, Some(left), Some(right), None) => emit_fast_native_binary(
            function,
            destination,
            left,
            right,
            kind,
            NumericOperator::Multiply,
            context,
        ),
        (NumericOperator::IntegerDivide, Some(left), Some(right), None) => {
            emit_guarded_division(function, destination, left, right, kind, context)
        }
        (NumericOperator::Remainder, Some(left), Some(right), None) => {
            emit_guarded_remainder(function, destination, left, right, kind, context)
        }
        (NumericOperator::Power, Some(base), Some(source_exponent), Some(power_scratch)) => {
            emit_integer_power_loop(
                function,
                IntegerPowerOperation {
                    destination,
                    base,
                    source_exponent,
                    kind,
                    scratch: power_scratch,
                    product: IntegerPowerProduct::Wrapping,
                },
                context,
            )
        }
        _ => Err(InfrastructureFailure::wasm_generation(format!(
            "Wasm fast integer operation has inconsistent operands for {operator:?}"
        ))),
    }
}

/// Emit one mod-2^N native binary operation and write the low bits to `destination`.
///
/// WHY: Wasm `i32`/`i64` add, subtract and multiply are exactly the language's modular
///      operations; unlike the checked path there is no predicate between the operands and the
///      destination store.
fn emit_fast_native_binary(
    function: &mut Function,
    destination: WasmLirLocalId,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    operator: NumericOperator,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), InfrastructureFailure> {
    emit_native_integer_binary(function, left, right, kind, operator, context)?;
    function.instruction(&Instruction::LocalSet(local_index(destination, context)?));
    Ok(())
}

/// How one integer power accumulation produces its next product.
///
/// WHY: checked power keeps every accumulated product inside the semantic domain before
///      comparing it, while fast power keeps only the low bits. The square-and-multiply loop is
///      otherwise identical, so the loop shape has one owner and only the product step varies.
#[derive(Clone, Copy)]
enum IntegerPowerProduct {
    /// Checked product, optionally using the widened I64 scratch for 32-bit multiply predicates.
    Checked {
        product_scratch: Option<WasmLirLocalId>,
    },
    /// Fast modular product with no overflow predicate.
    Wrapping,
}

impl IntegerPowerProduct {
    fn emit(
        self,
        function: &mut Function,
        destination: WasmLirLocalId,
        left: WasmLirLocalId,
        right: WasmLirLocalId,
        kind: WasmIntegerOperationKind,
        context: &LirBodyEmitContext<'_>,
    ) -> Result<(), InfrastructureFailure> {
        match self {
            Self::Checked { product_scratch } => emit_checked_multiply_into(
                function,
                destination,
                left,
                right,
                kind,
                product_scratch,
                context,
            ),
            Self::Wrapping => {
                emit_wrapping_integer_multiply(function, destination, left, right, kind, context)
            }
        }
    }
}

/// The shared input of checked and fast integer power emission.
///
/// WHY: the two operands, result, carrier, loop scratch and product policy travel together
///      through one square-and-multiply pass, so they form one operation record rather than six
///      positional parameters.
#[derive(Clone, Copy)]
struct IntegerPowerOperation {
    destination: WasmLirLocalId,
    base: WasmLirLocalId,
    source_exponent: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    scratch: WasmIntegerPowerScratch,
    product: IntegerPowerProduct,
}

/// Emits the signed-exponent guard and square-and-multiply loop shared by integer power.
///
/// WHAT: traps a negative signed exponent, then multiplies products selected by the exponent's
///       low bit while shifting it right until zero.
/// WHY: raising the value before truncation would construct a huge power and lose a modular
///      result. `0 ^ 0` stays `1` because the accumulator starts there and the loop body never
///      runs for a zero exponent.
fn emit_integer_power_loop(
    function: &mut Function,
    operation: IntegerPowerOperation,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), InfrastructureFailure> {
    let IntegerPowerOperation {
        destination,
        base,
        source_exponent,
        kind,
        scratch,
        product,
    } = operation;
    let WasmIntegerPowerScratch { factor, exponent } = scratch;

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
    // A trap aborts before the caller observes this result, so the accumulator may live here.
    emit_integer_constant(function, kind, 1);
    function.instruction(&Instruction::LocalSet(local_index(destination, context)?));

    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    emit_local_is_zero(function, exponent, kind, context)?;
    function.instruction(&Instruction::BrIf(1));

    emit_local_is_odd(function, exponent, kind, context)?;
    function.instruction(&Instruction::If(BlockType::Empty));
    product.emit(function, destination, destination, factor, kind, context)?;
    function.instruction(&Instruction::End);

    // An unused final factor square must not introduce an overflow past the result.
    emit_shift_exponent_right(function, exponent, kind, context)?;
    emit_local_is_zero(function, exponent, kind, context)?;
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::If(BlockType::Empty));
    product.emit(function, factor, factor, factor, kind, context)?;
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::Br(0));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
    Ok(())
}

/// Emits unary negation as the modular `0 - operand` with no minimum predicate.
///
/// WHY: proven-safe negation excludes the signed minimum by analysis, and fast negation wraps
///      that minimum to itself, so neither family needs the runtime-checked minimum predicate.
///      Unsigned negation has no language domain at all.
fn emit_wrapping_negation(
    function: &mut Function,
    destination: WasmLirLocalId,
    operand: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), InfrastructureFailure> {
    if signed_bounds(kind).is_none() {
        return Err(InfrastructureFailure::wasm_generation(
            "Wasm modular negation requires a signed integer domain".to_owned(),
        ));
    }

    emit_integer_constant(function, kind, 0);
    function.instruction(&Instruction::LocalGet(local_index(operand, context)?));
    function.instruction(integer_opcode(kind, NumericOperator::Subtract)?);
    function.instruction(&Instruction::LocalSet(local_index(destination, context)?));
    Ok(())
}

/// Emit one modular multiply without any overflow predicate.
fn emit_wrapping_integer_multiply(
    function: &mut Function,
    destination: WasmLirLocalId,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), InfrastructureFailure> {
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

fn emit_checked_add(
    function: &mut Function,
    destination: WasmLirLocalId,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), InfrastructureFailure> {
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
) -> Result<(), InfrastructureFailure> {
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
) -> Result<(), InfrastructureFailure> {
    let Some((minimum, _)) = signed_bounds(kind) else {
        return Err(InfrastructureFailure::wasm_generation(
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

/// Emits truncating division with its zero-divisor and signed `MIN // -1` guards.
///
/// WHY: runtime-checked ordinary division and `$fast_math` division report exactly the same
///      faults, so one guard sequence serves both. Wasm delivers either failure as
///      `unreachable`, which keeps one Moth trap transport and does not depend on the engine's
///      `div_s` trap message.
fn emit_guarded_division(
    function: &mut Function,
    destination: WasmLirLocalId,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), InfrastructureFailure> {
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

/// Emits dividend-sign remainder with its zero-divisor guard.
///
/// WHY: runtime-checked ordinary remainder and `$fast_math` remainder report only a zero
///      divisor, and Wasm `rem_s`/`rem_u` are defined to return zero for `MIN % -1` without
///      trapping, so the signed minimum needs no branch.
fn emit_guarded_remainder(
    function: &mut Function,
    destination: WasmLirLocalId,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), InfrastructureFailure> {
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

fn emit_checked_multiply_into(
    function: &mut Function,
    destination: WasmLirLocalId,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    product_scratch: Option<WasmLirLocalId>,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), InfrastructureFailure> {
    match kind {
        WasmIntegerOperationKind::Signed32 | WasmIntegerOperationKind::Unsigned32 => {
            let product_scratch = product_scratch.ok_or_else(|| {
                InfrastructureFailure::wasm_generation(
                    "Wasm 32-bit multiply is missing its I64 product scratch".to_owned(),
                )
            })?;
            let extend = match kind {
                WasmIntegerOperationKind::Signed32 => &Instruction::I64ExtendI32S,
                WasmIntegerOperationKind::Unsigned32 => &Instruction::I64ExtendI32U,
                _ => crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "32-bit multiply arm is entered only for Signed32 or Unsigned32 at emit_checked_multiply_into"
                ),
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
                _ => crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "32-bit multiply arm is entered only for Signed32 or Unsigned32 at emit_checked_multiply_into"
                ),
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
                crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "Wasm 64-bit multiply has unexpected product scratch"
                );
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
) -> Result<(), InfrastructureFailure> {
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
        _ => Err(InfrastructureFailure::wasm_generation(
            "Wasm widened multiply check requires a 64-bit integer domain".to_owned(),
        )),
    }
}

fn emit_signed_multiply_overflow_check(
    function: &mut Function,
    left: WasmLirLocalId,
    right: WasmLirLocalId,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), InfrastructureFailure> {
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
) -> Result<(), InfrastructureFailure> {
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
) -> Result<(), InfrastructureFailure> {
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
) -> Result<(), InfrastructureFailure> {
    function.instruction(&Instruction::LocalGet(local_index(left, context)?));
    function.instruction(&Instruction::LocalGet(local_index(right, context)?));
    function.instruction(match kind {
        WasmIntegerOperationKind::Unsigned32 => &Instruction::I32LtU,
        WasmIntegerOperationKind::Unsigned64 => &Instruction::I64LtU,
        _ => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "emit_unsigned_less_than is only called for Unsigned32 or Unsigned64 from emit_checked_subtract"
        ),
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
        _ => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm integer operation carriers are only I32 or I64"
        ),
    });
}

fn emit_local_equals_constant(
    function: &mut Function,
    local: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    value: i64,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), InfrastructureFailure> {
    function.instruction(&Instruction::LocalGet(local_index(local, context)?));
    emit_integer_constant(function, kind, value);
    function.instruction(match kind.carrier() {
        WasmAbiType::I32 => &Instruction::I32Eq,
        WasmAbiType::I64 => &Instruction::I64Eq,
        _ => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm integer operation carriers are only I32 or I64"
        ),
    });
    Ok(())
}

fn emit_local_is_zero(
    function: &mut Function,
    local: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), InfrastructureFailure> {
    function.instruction(&Instruction::LocalGet(local_index(local, context)?));
    function.instruction(match kind.carrier() {
        WasmAbiType::I32 => &Instruction::I32Eqz,
        WasmAbiType::I64 => &Instruction::I64Eqz,
        _ => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm integer operation carriers are only I32 or I64"
        ),
    });
    Ok(())
}

fn emit_local_is_odd(
    function: &mut Function,
    local: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), InfrastructureFailure> {
    function.instruction(&Instruction::LocalGet(local_index(local, context)?));
    emit_integer_constant(function, kind, 1);
    function.instruction(match kind.carrier() {
        WasmAbiType::I32 => &Instruction::I32And,
        WasmAbiType::I64 => &Instruction::I64And,
        _ => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm integer operation carriers are only I32 or I64"
        ),
    });
    emit_integer_constant(function, kind, 0);
    function.instruction(match kind.carrier() {
        WasmAbiType::I32 => &Instruction::I32Ne,
        WasmAbiType::I64 => &Instruction::I64Ne,
        _ => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm integer operation carriers are only I32 or I64"
        ),
    });
    Ok(())
}

fn emit_shift_exponent_right(
    function: &mut Function,
    exponent: WasmLirLocalId,
    kind: WasmIntegerOperationKind,
    context: &LirBodyEmitContext<'_>,
) -> Result<(), InfrastructureFailure> {
    function.instruction(&Instruction::LocalGet(local_index(exponent, context)?));
    emit_integer_constant(function, kind, 1);
    function.instruction(match kind.carrier() {
        WasmAbiType::I32 => &Instruction::I32ShrU,
        WasmAbiType::I64 => &Instruction::I64ShrU,
        _ => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm integer operation carriers are only I32 or I64"
        ),
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
) -> Result<(), InfrastructureFailure> {
    function.instruction(&Instruction::LocalGet(local_index(left, context)?));
    function.instruction(&Instruction::LocalGet(local_index(right, context)?));
    function.instruction(integer_opcode(kind, operator)?);
    Ok(())
}

fn integer_opcode(
    kind: WasmIntegerOperationKind,
    operator: NumericOperator,
) -> Result<&'static Instruction<'static>, InfrastructureFailure> {
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
            return Err(InfrastructureFailure::wasm_generation(format!(
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
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "Wasm integer operation carriers are only I32 or I64"
            )
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
