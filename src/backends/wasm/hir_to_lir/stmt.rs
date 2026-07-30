//! Statement lowering for HIR -> Wasm LIR.

use crate::backends::error_types::lir_transformation_error;
use crate::backends::wasm::hir_to_lir::context::WasmFunctionLoweringContext;
use crate::backends::wasm::hir_to_lir::expr::lower_expression;
use crate::backends::wasm::hir_to_lir::imports::resolve_host_call_import;
use crate::backends::wasm::lir::instructions::{WasmCalleeRef, WasmLirStmt};
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expressions::HirExpression;
use crate::compiler_frontend::hir::ids::LocalId;
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
        HirStatementKind::NumericOp { op, .. } => Err(lir_transformation_error(format!(
            "Wasm lowering does not yet support checked numeric operations: {op:?}"
        ))),
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

fn lower_assignment(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    target: &HirPlace,
    value: &crate::compiler_frontend::hir::expressions::HirExpression,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), CompilerError> {
    // WHAT: preserve explicit move/copy distinction in LIR.
    // WHY: ownership optimization stays representable even under GC-first semantics.
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
