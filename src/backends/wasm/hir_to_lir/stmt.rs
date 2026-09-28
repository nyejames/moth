//! Statement lowering for HIR -> Wasm LIR.

use crate::backends::error_types::lir_transformation_error;
use crate::backends::wasm::hir_to_lir::context::WasmFunctionLoweringContext;
use crate::backends::wasm::hir_to_lir::expr::lower_expression;
use crate::backends::wasm::hir_to_lir::imports::resolve_host_call_import;
use crate::backends::wasm::lir::instructions::{
    WasmCalleeRef, WasmIntegerOperationKind, WasmIntegerOperationOperands, WasmIntegerPowerScratch,
    WasmIntegerScratch, WasmLirStmt,
};
use crate::backends::wasm::lir::types::WasmAbiType;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_profile::NumericProfile;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
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
        } => lower_checked_integer_operation(
            context,
            *op,
            *failure_mode,
            operands,
            *result,
            statements,
        ),
        HirStatementKind::FormatFloat { .. } => Err(lir_transformation_error(
            "Wasm lowering does not yet support Float formatting",
        )),
        HirStatementKind::ValidateFloat { .. } => Err(lir_transformation_error(
            "Wasm lowering does not yet support Float boundary validation",
        )),
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
            WasmIntegerOperationOperands::Unary {
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
            WasmIntegerOperationOperands::Binary {
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
        WasmIntegerOperationOperands::Unary { operand } => destination == operand,
        WasmIntegerOperationOperands::Binary { left, right } => {
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
