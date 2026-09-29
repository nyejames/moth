//! Statement lowering for HIR -> Wasm LIR.

use crate::backends::error_types::lir_transformation_error;
use crate::backends::wasm::hir_to_lir::context::WasmFunctionLoweringContext;
use crate::backends::wasm::hir_to_lir::expr::lower_expression;
use crate::backends::wasm::hir_to_lir::imports::resolve_host_call_import;
use crate::backends::wasm::lir::instructions::{
    WasmCalleeRef, WasmIntegerOperationKind, WasmIntegerPowerScratch, WasmIntegerScratch,
    WasmLirStmt, WasmNumericOperationOperands,
};
use crate::backends::wasm::lir::types::WasmAbiType;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_profile::NumericProfile;
use crate::compiler_frontend::datatypes::numeric_scalar::{BinaryFloatPrecision, NumericScalar};
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expressions::HirExpression;
use crate::compiler_frontend::hir::ids::LocalId;
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};

pub(crate) fn lower_statement(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    statement: &HirStatement,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), CompilerError> {
    // Statement lowering is explicitly side-effecting: expressions append LIR
    // statements directly to preserve HIR evaluation order.
    match &statement.kind {
        HirStatementKind::Assign { target, value } => {
            lower_assignment(context, target, value, statements)
        }
        HirStatementKind::Call {
            target,
            args,
            result,
        } => {
            let mut lowered_args = Vec::with_capacity(args.len());
            for arg in args {
                let lowered = lower_expression(context, arg, statements)?;
                lowered_args.push(lowered.value);
            }

            let callee = match target {
                CallTarget::Local(function_id) => {
                    // User calls stay function-id based after semantic lowering.
                    let function_id = context
                        .module_context
                        .function_map
                        .get(function_id)
                        .copied()
                        .ok_or_else(|| {
                            lir_transformation_error(format!(
                                "Wasm lowering missing function id mapping for {function_id:?}"
                            ))
                        })?;
                    WasmCalleeRef::Function(function_id)
                }
                CallTarget::CrossModule(origin) => {
                    return Err(lir_transformation_error(format!(
                        "Wasm lowering received unresolved cross-module function target {origin:?}"
                    )));
                }
                CallTarget::ModulePrivate(identity) => {
                    return Err(lir_transformation_error(format!(
                        "Wasm lowering received unresolved module-private function target {identity:?}"
                    )));
                }
                CallTarget::Generated(identity) => {
                    return Err(lir_transformation_error(format!(
                        "Wasm lowering received unresolved generated function target {identity:?}"
                    )));
                }
                CallTarget::External(_) => {
                    // Host calls lower to deterministic import ids.
                    let import_id = resolve_host_call_import(context.module_context, target)?;
                    WasmCalleeRef::Import(import_id)
                }
            };

            let dst = result
                .as_ref()
                .and_then(|local_id| context.local_map.get(local_id).copied());

            statements.push(WasmLirStmt::Call {
                dst,
                callee,
                args: lowered_args,
            });

            Ok(())
        }
        HirStatementKind::MapOp { .. } => Err(lir_transformation_error(
            "Wasm hashmap operation reached lowering before backend feature validation",
        )),
        HirStatementKind::CastOp { .. } => Err(lir_transformation_error(
            "Wasm lowering does not yet support cast operations",
        )),
        HirStatementKind::NumericOp {
            op,
            failure_mode,
            operands,
            result,
        } => {
            if op.domain.is_integer() {
                lower_checked_integer_operation(
                    context,
                    *op,
                    *failure_mode,
                    operands,
                    *result,
                    statements,
                )
            } else {
                lower_checked_float_operation(
                    context,
                    *op,
                    *failure_mode,
                    operands,
                    *result,
                    statements,
                )
            }
        }
        HirStatementKind::FloatRangeCandidate {
            current,
            step,
            end,
            ascending,
            inclusive,
            domain,
            candidate_result,
            in_range_result,
        } => lower_float_range_candidate(
            context,
            [current, step, end, ascending],
            *inclusive,
            *domain,
            [*candidate_result, *in_range_result],
            statements,
        ),
        HirStatementKind::FormatFloat {
            source,
            failure_mode,
            result,
        } => lower_format_float(context, source, *failure_mode, *result, statements),
        HirStatementKind::ValidateFloat {
            source,
            failure_mode,
            result,
        } => lower_validate_float(context, source, *failure_mode, *result, statements),
        HirStatementKind::Expr(expression) => {
            let _ = lower_expression(context, expression, statements)?;
            Ok(())
        }
        HirStatementKind::Drop(local_id) => {
            // Keep explicit source-level drops in LIR when the value is handle-like.
            let mapped_local = context.local_map.get(local_id).copied().ok_or_else(|| {
                lir_transformation_error(format!(
                    "Wasm lowering could not resolve drop local {local_id:?}"
                ))
            })?;

            if context.is_handle_local(mapped_local) {
                statements.push(WasmLirStmt::DropIfOwned {
                    value: mapped_local,
                });
            }

            Ok(())
        }

        HirStatementKind::PushRuntimeFragment { vec_local, value } => {
            // WHAT: lower a runtime fragment push into a Wasm vec-push sequence.
            // WHY: entry start() accumulates runtime fragments via PushRuntimeFragment;
            //      the Wasm backend must append the evaluated string to the fragment vec.
            lower_push_runtime_fragment(context, vec_local, value, statements)
        }
    }
}

fn lower_checked_integer_operation(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    op: HirNumericOp,
    failure_mode: NumericFailureMode,
    operands: &HirNumericOperands,
    result: LocalId,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), CompilerError> {
    if failure_mode != NumericFailureMode::Trap || !op.domain.is_integer() {
        return Err(lir_transformation_error(format!(
            "Wasm lowering does not support checked numeric operation {op} in {failure_mode:?} mode"
        )));
    }

    let profile = context.module_context.request.numeric_profile;
    let kind = integer_operation_kind(op.domain, profile)?;
    let lir_operands = match (op.operator, operands) {
        (NumericOperator::Negate, HirNumericOperands::Unary { operand }) => {
            let lowered = lower_expression(context, operand, statements)?;
            WasmNumericOperationOperands::Unary {
                operand: lowered.value,
            }
        }
        (
            NumericOperator::Add
            | NumericOperator::Subtract
            | NumericOperator::Multiply
            | NumericOperator::IntegerDivide
            | NumericOperator::Remainder
            | NumericOperator::Power,
            HirNumericOperands::Binary { left, right },
        ) => {
            let lowered_left = lower_expression(context, left, statements)?;
            let lowered_right = lower_expression(context, right, statements)?;
            WasmNumericOperationOperands::Binary {
                left: lowered_left.value,
                right: lowered_right.value,
            }
        }
        _ => {
            return Err(lir_transformation_error(format!(
                "Wasm lowering received invalid integer numeric operands for {op}"
            )));
        }
    };

    let Some(destination) = context.local_map.get(&result).copied() else {
        return Err(lir_transformation_error(format!(
            "Wasm lowering could not resolve numeric result local {result:?}"
        )));
    };
    let expected_carrier = kind.carrier();
    let destination_carrier = context.local_type_by_id.get(&destination).copied();
    if destination_carrier != Some(expected_carrier) {
        return Err(lir_transformation_error(format!(
            "Wasm lowering expected numeric result carrier {expected_carrier:?}, found {destination_carrier:?}"
        )));
    }

    // Generated loop updates can reuse an operand's HIR local. Keep its old value available to
    // overflow checks, then commit the assignment only after the checked operation succeeds.
    let destination_aliases_source = match lir_operands {
        WasmNumericOperationOperands::Unary { operand } => destination == operand,
        WasmNumericOperationOperands::Binary { left, right } => {
            destination == left || destination == right
        }
    };
    let operation_destination = if destination_aliases_source {
        context.alloc_temp(expected_carrier)
    } else {
        destination
    };

    let needs_product_scratch = matches!(
        kind,
        WasmIntegerOperationKind::Signed32 | WasmIntegerOperationKind::Unsigned32
    ) && matches!(
        op.operator,
        NumericOperator::Multiply | NumericOperator::Power
    );
    let product_scratch = if needs_product_scratch {
        Some(context.alloc_temp(WasmAbiType::I64))
    } else {
        None
    };
    let power_scratch = if op.operator == NumericOperator::Power {
        Some(WasmIntegerPowerScratch {
            factor: context.alloc_temp(expected_carrier),
            exponent: context.alloc_temp(expected_carrier),
        })
    } else {
        None
    };

    let scratch = WasmIntegerScratch {
        product: product_scratch,
        power: power_scratch,
    };
    statements.push(WasmLirStmt::CheckedIntegerOp {
        dst: operation_destination,
        operator: op.operator,
        kind,
        operands: lir_operands,
        scratch,
    });

    if destination_aliases_source {
        statements.push(WasmLirStmt::Copy {
            dst: destination,
            src: operation_destination,
        });
    }

    Ok(())
}

fn lower_validate_float(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    source: &HirExpression,
    failure_mode: NumericFailureMode,
    result: LocalId,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), CompilerError> {
    if failure_mode != NumericFailureMode::Trap {
        return Err(lir_transformation_error(format!(
            "Wasm lowering does not support Float validation in {failure_mode:?} mode"
        )));
    }

    let type_environment = context.module_context.type_environment;
    let float_type = type_environment.builtins().float;
    if source.ty != float_type {
        return Err(lir_transformation_error(format!(
            "Wasm Float validation source has type {:?}, expected Float",
            source.ty
        )));
    }

    let precision = NumericScalar::Float
        .binary_float_precision(context.module_context.request.numeric_profile)
        .filter(|precision| {
            matches!(
                precision,
                BinaryFloatPrecision::Binary32 | BinaryFloatPrecision::Binary64
            )
        })
        .ok_or_else(|| {
            lir_transformation_error("Wasm Float validation requires F32 or F64 precision")
        })?;
    let expected_carrier = match precision {
        BinaryFloatPrecision::Binary32 => WasmAbiType::F32,
        BinaryFloatPrecision::Binary64 => WasmAbiType::F64,
        BinaryFloatPrecision::Binary16 => unreachable!("F16 precision was filtered above"),
    };

    let destination = context.local_map.get(&result).copied().ok_or_else(|| {
        lir_transformation_error(format!(
            "Wasm lowering could not resolve Float validation result local {result:?}"
        ))
    })?;
    if context.local_type_by_id.get(&destination).copied() != Some(expected_carrier) {
        return Err(lir_transformation_error(format!(
            "Wasm Float validation result does not use the profile carrier {expected_carrier:?}"
        )));
    }

    // Lower the boundary value once; ValidateFloat's emitter reads it before writing the result,
    // so an in-place result can reuse the source local without a copy.
    let lowered_source = lower_expression(context, source, statements)?;
    if context.local_type_by_id.get(&lowered_source.value).copied() != Some(expected_carrier) {
        return Err(lir_transformation_error(format!(
            "Wasm Float validation source does not use the profile carrier {expected_carrier:?}"
        )));
    }

    statements.push(WasmLirStmt::ValidateFloat {
        dst: destination,
        source: lowered_source.value,
        precision,
    });
    Ok(())
}

fn lower_format_float(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    source: &HirExpression,
    failure_mode: NumericFailureMode,
    result: LocalId,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), CompilerError> {
    if failure_mode != NumericFailureMode::Trap {
        return Err(lir_transformation_error(format!(
            "Wasm lowering does not support Float formatting in {failure_mode:?} mode"
        )));
    }

    let type_environment = context.module_context.type_environment;
    if source.ty != type_environment.builtins().float {
        return Err(lir_transformation_error(format!(
            "Wasm Float formatting source has type {:?}, expected Float",
            source.ty
        )));
    }

    let precision = NumericScalar::Float
        .binary_float_precision(context.module_context.request.numeric_profile)
        .filter(|precision| {
            matches!(
                precision,
                BinaryFloatPrecision::Binary32 | BinaryFloatPrecision::Binary64
            )
        })
        .ok_or_else(|| {
            lir_transformation_error("Wasm Float formatting requires F32 or F64 precision")
        })?;
    let expected_carrier = match precision {
        BinaryFloatPrecision::Binary32 => WasmAbiType::F32,
        BinaryFloatPrecision::Binary64 => WasmAbiType::F64,
        BinaryFloatPrecision::Binary16 => unreachable!("F16 precision was filtered above"),
    };

    let destination = context.local_map.get(&result).copied().ok_or_else(|| {
        lir_transformation_error(format!(
            "Wasm lowering could not resolve Float formatting result local {result:?}"
        ))
    })?;
    if context.local_type_by_id.get(&destination).copied() != Some(WasmAbiType::Handle) {
        return Err(lir_transformation_error(
            "Wasm Float formatting result does not use the String handle carrier",
        ));
    }

    // Evaluate the source once; its semantic Float precision remains explicit in LIR.
    let lowered_source = lower_expression(context, source, statements)?;
    if context.local_type_by_id.get(&lowered_source.value).copied() != Some(expected_carrier) {
        return Err(lir_transformation_error(format!(
            "Wasm Float formatting source does not use the profile carrier {expected_carrier:?}"
        )));
    }

    statements.push(WasmLirStmt::StringFromFloat {
        dst: destination,
        value: lowered_source.value,
        precision,
    });
    Ok(())
}

fn lower_checked_float_operation(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    op: HirNumericOp,
    failure_mode: NumericFailureMode,
    operands: &HirNumericOperands,
    result: LocalId,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), CompilerError> {
    if failure_mode != NumericFailureMode::Trap || !op.domain.is_binary_float() {
        return Err(lir_transformation_error(format!(
            "Wasm lowering does not support checked float operation {op} in {failure_mode:?} mode"
        )));
    }

    let profile = context.module_context.request.numeric_profile;
    let precision = op
        .domain
        .binary_float_precision(profile)
        .filter(|precision| {
            matches!(
                precision,
                BinaryFloatPrecision::Binary32 | BinaryFloatPrecision::Binary64
            )
        })
        .ok_or_else(|| {
            lir_transformation_error(format!(
                "Wasm checked float lowering does not support {} precision",
                op.domain.name()
            ))
        })?;
    let expected_type = op.domain.type_id(context.module_context.type_environment);

    let lir_operands = match (op.operator, operands) {
        (NumericOperator::Negate, HirNumericOperands::Unary { operand }) => {
            if operand.ty != expected_type {
                return Err(lir_transformation_error(format!(
                    "Wasm checked float operand has type {:?}, expected {}",
                    operand.ty,
                    op.domain.name()
                )));
            }
            let lowered = lower_expression(context, operand, statements)?;
            WasmNumericOperationOperands::Unary {
                operand: lowered.value,
            }
        }
        (
            NumericOperator::Add
            | NumericOperator::Subtract
            | NumericOperator::Multiply
            | NumericOperator::Divide
            | NumericOperator::Remainder
            | NumericOperator::Power,
            HirNumericOperands::Binary { left, right },
        ) => {
            if left.ty != expected_type || right.ty != expected_type {
                return Err(lir_transformation_error(format!(
                    "Wasm checked float operands do not match {}",
                    op.domain.name()
                )));
            }

            // Linearise both operands in source order before emitting the checked operation.
            let lowered_left = lower_expression(context, left, statements)?;
            let lowered_right = lower_expression(context, right, statements)?;
            WasmNumericOperationOperands::Binary {
                left: lowered_left.value,
                right: lowered_right.value,
            }
        }
        _ => {
            return Err(lir_transformation_error(format!(
                "Wasm lowering received invalid float numeric operands for {op}"
            )));
        }
    };

    let Some(destination) = context.local_map.get(&result).copied() else {
        return Err(lir_transformation_error(format!(
            "Wasm lowering could not resolve float numeric result local {result:?}"
        )));
    };
    let expected_carrier = match precision {
        BinaryFloatPrecision::Binary32 => WasmAbiType::F32,
        BinaryFloatPrecision::Binary64 => WasmAbiType::F64,
        BinaryFloatPrecision::Binary16 => unreachable!("F16 precision was filtered above"),
    };
    if context.local_type_by_id.get(&destination).copied() != Some(expected_carrier) {
        return Err(lir_transformation_error(format!(
            "Wasm lowering expected float result carrier {expected_carrier:?}"
        )));
    }

    // The emitter consumes all operand locals before storing the result, so in-place HIR updates
    // need neither a copy nor a scratch local.
    statements.push(WasmLirStmt::CheckedFloatOp {
        dst: destination,
        operator: op.operator,
        precision,
        operands: lir_operands,
    });

    Ok(())
}

fn lower_float_range_candidate(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    inputs: [&HirExpression; 4],
    inclusive: bool,
    domain: NumericScalar,
    output_ids: [LocalId; 2],
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), CompilerError> {
    let [current, step, end, ascending] = inputs;
    let [candidate_result, in_range_result] = output_ids;
    let profile = context.module_context.request.numeric_profile;
    let precision = domain
        .binary_float_precision(profile)
        .filter(|precision| {
            matches!(
                precision,
                BinaryFloatPrecision::Binary32 | BinaryFloatPrecision::Binary64
            )
        })
        .ok_or_else(|| {
            lir_transformation_error(format!(
                "Wasm float range candidates do not support {} precision",
                domain.name()
            ))
        })?;
    let expected_type = domain.type_id(context.module_context.type_environment);
    if current.ty != expected_type || step.ty != expected_type || end.ty != expected_type {
        return Err(lir_transformation_error(format!(
            "Wasm float range candidate operands do not match {}",
            domain.name()
        )));
    }
    if ascending.ty != context.module_context.type_environment.builtins().bool {
        return Err(lir_transformation_error(
            "Wasm float range candidate direction must be Bool",
        ));
    }

    let expected_carrier = match precision {
        BinaryFloatPrecision::Binary32 => WasmAbiType::F32,
        BinaryFloatPrecision::Binary64 => WasmAbiType::F64,
        BinaryFloatPrecision::Binary16 => unreachable!("F16 precision was filtered above"),
    };
    let lowered_current = lower_expression(context, current, statements)?;
    let lowered_step = lower_expression(context, step, statements)?;
    let lowered_end = lower_expression(context, end, statements)?;
    let lowered_ascending = lower_expression(context, ascending, statements)?;
    for (operand, label) in [
        (lowered_current.value, "current"),
        (lowered_step.value, "step"),
        (lowered_end.value, "end"),
    ] {
        if context.local_type_by_id.get(&operand).copied() != Some(expected_carrier) {
            return Err(lir_transformation_error(format!(
                "Wasm float range candidate {label} does not use carrier {expected_carrier:?}"
            )));
        }
    }
    if context
        .local_type_by_id
        .get(&lowered_ascending.value)
        .copied()
        != Some(WasmAbiType::I32)
    {
        return Err(lir_transformation_error(
            "Wasm float range candidate direction does not use the I32 Bool carrier",
        ));
    }

    let candidate_dst = context
        .local_map
        .get(&candidate_result)
        .copied()
        .ok_or_else(|| {
            lir_transformation_error(format!(
                "Wasm lowering could not resolve float range candidate local {candidate_result:?}"
            ))
        })?;
    let in_range_dst = context
        .local_map
        .get(&in_range_result)
        .copied()
        .ok_or_else(|| {
            lir_transformation_error(format!(
                "Wasm lowering could not resolve float range Bool local {in_range_result:?}"
            ))
        })?;
    if context.local_type_by_id.get(&candidate_dst).copied() != Some(expected_carrier) {
        return Err(lir_transformation_error(format!(
            "Wasm float range candidate destination does not use carrier {expected_carrier:?}"
        )));
    }
    if context.local_type_by_id.get(&in_range_dst).copied() != Some(WasmAbiType::I32) {
        return Err(lir_transformation_error(
            "Wasm float range Bool result does not use the I32 carrier",
        ));
    }

    let scratch = context.alloc_temp(expected_carrier);
    statements.push(WasmLirStmt::FloatRangeCandidate {
        candidate_dst,
        in_range_dst,
        scratch,
        current: lowered_current.value,
        step: lowered_step.value,
        end: lowered_end.value,
        ascending: lowered_ascending.value,
        precision,
        inclusive,
    });
    Ok(())
}

fn integer_operation_kind(
    domain: NumericScalar,
    profile: NumericProfile,
) -> Result<WasmIntegerOperationKind, CompilerError> {
    let Some((minimum, maximum)) = domain.integer_range(profile) else {
        return Err(lir_transformation_error(format!(
            "Wasm checked integer lowering received a non-integer domain {}",
            domain.name()
        )));
    };

    let kind = match (minimum, maximum) {
        (minimum, maximum)
            if minimum == i128::from(i32::MIN) && maximum == i128::from(i32::MAX) =>
        {
            WasmIntegerOperationKind::Signed32
        }
        (minimum, maximum) if minimum == 0 && maximum == i128::from(u32::MAX) => {
            WasmIntegerOperationKind::Unsigned32
        }
        (minimum, maximum)
            if minimum == i128::from(i64::MIN) && maximum == i128::from(i64::MAX) =>
        {
            WasmIntegerOperationKind::Signed64
        }
        (minimum, maximum) if minimum == 0 && maximum == i128::from(u64::MAX) => {
            WasmIntegerOperationKind::Unsigned64
        }
        _ => {
            return Err(lir_transformation_error(format!(
                "Wasm checked integer domain {} does not match an exact I32/U32/I64/U64 range",
                domain.name()
            )));
        }
    };

    Ok(kind)
}

fn lower_assignment(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    target: &HirPlace,
    value: &crate::compiler_frontend::hir::expressions::HirExpression,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), CompilerError> {
    // WHAT: preserve explicit move/copy distinction in LIR.
    // WHY: the current emitter keeps the ownership hook representable while transitional
    // collected scaffolding remains. Final lowering consumes `ValidatedMemoryPlan`.
    let HirPlace::Local(target_local) = target else {
        return Err(lir_transformation_error(
            "Wasm lowering currently supports assignments only to direct locals",
        ));
    };

    let dst = context
        .local_map
        .get(target_local)
        .copied()
        .ok_or_else(|| {
            lir_transformation_error(format!(
                "Wasm lowering could not resolve assignment target local {target_local:?}",
            ))
        })?;

    let lowered = lower_expression(context, value, statements)?;
    if lowered.value == dst {
        return Ok(());
    }

    if lowered.prefer_move {
        statements.push(WasmLirStmt::Move {
            dst,
            src: lowered.value,
        });
    } else {
        statements.push(WasmLirStmt::Copy {
            dst,
            src: lowered.value,
        });
    }

    Ok(())
}

fn lower_push_runtime_fragment(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    vec_local: &LocalId,
    value: &HirExpression,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), CompilerError> {
    let vec_handle = context.local_map.get(vec_local).copied().ok_or_else(|| {
        lir_transformation_error(format!(
            "Wasm lowering could not resolve runtime fragment vec local {vec_local:?}",
        ))
    })?;

    if !context.is_handle_local(vec_handle) {
        return Err(lir_transformation_error(format!(
            "Wasm lowering expected runtime fragment vec local {vec_local:?} to lower as a handle",
        )));
    }

    let lowered_value = lower_expression(context, value, statements)?;
    if !context.is_handle_local(lowered_value.value) {
        return Err(lir_transformation_error(
            "Wasm lowering expected runtime fragment values to lower as string handles",
        ));
    }

    statements.push(WasmLirStmt::VecPushHandle {
        vec: vec_handle,
        handle: lowered_value.value,
    });

    Ok(())
}
