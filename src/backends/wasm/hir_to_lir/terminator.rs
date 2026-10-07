//! Terminator lowering for HIR -> Wasm LIR.

use crate::backends::error_types::lir_transformation_error;
use crate::backends::wasm::hir_to_lir::context::{WasmFunctionLoweringContext, lower_type_to_abi};
use crate::backends::wasm::hir_to_lir::expr::{lower_concrete_string, lower_expression};
use crate::backends::wasm::hir_to_lir::imports::ensure_host_import;
use crate::backends::wasm::lir::instructions::{WasmCalleeRef, WasmLirStmt, WasmLirTerminator};
use crate::backends::wasm::lir::types::WasmAbiType;
use crate::backends::wasm::runtime::imports::WasmHostFunction;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, HirVariantCarrier};
use crate::compiler_frontend::hir::ids::BlockId;
use crate::compiler_frontend::hir::terminators::{
    HirAssertionMessageEvaluation, HirJumpArgument, HirTerminator,
};
use std::collections::HashSet;

pub(crate) fn lower_terminator(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    terminator: &HirTerminator,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<WasmLirTerminator, CompilerError> {
    // Unsupported high-level control-flow forms intentionally error here so lowering failures are
    // structured and visible while Wasm control-flow support remains experimental.
    match terminator {
        HirTerminator::Jump { target, args } => {
            let target = lower_jump_argument_transfer(context, *target, args, statements)?;
            Ok(WasmLirTerminator::Jump(target))
        }
        HirTerminator::Break { target } | HirTerminator::Continue { target } => {
            Ok(WasmLirTerminator::Jump(resolve_block_id(context, *target)?))
        }
        HirTerminator::If {
            condition,
            then_block,
            else_block,
        } => {
            let lowered_condition = lower_expression(context, *condition, statements)?;
            Ok(WasmLirTerminator::Branch {
                condition: lowered_condition.value,
                then_block: resolve_block_id(context, *then_block)?,
                else_block: resolve_block_id(context, *else_block)?,
            })
        }
        HirTerminator::Return(value) => {
            // Preserve unit-return as `Return(None)` to keep ABI shape explicit.
            let return_abi = lower_type_to_abi(
                context.module_context,
                context
                    .module_context
                    .hir_module
                    .expressions
                    .expression(*value)
                    .ty,
            );
            if matches!(return_abi, WasmAbiType::Void) {
                return Ok(WasmLirTerminator::Return { value: None });
            }

            let lowered_value = lower_expression(context, *value, statements)?;
            Ok(WasmLirTerminator::Return {
                value: Some(lowered_value.value),
            })
        }
        HirTerminator::ReturnSuccess(_) => Err(lir_transformation_error(
            "Wasm lowering does not yet support fallible success-return terminators",
        )),
        HirTerminator::ReturnError(_) => Err(lir_transformation_error(
            "Wasm lowering does not yet support fallible error-return terminators",
        )),
        HirTerminator::FallibleBranch { .. } => Err(lir_transformation_error(
            "Wasm lowering does not yet support fallible branch terminators",
        )),
        HirTerminator::Uninitialized => Err(lir_transformation_error(
            "Wasm lowering encountered Uninitialized terminator",
        )),
        HirTerminator::RuntimeFailure { .. } => Ok(WasmLirTerminator::Trap),
        HirTerminator::AssertFailure {
            message,
            message_evaluation,
        } => {
            // The folded option's payload is already evaluated. Lower only its String, not the
            // unsupported general variant payload representation.
            let message = match message_evaluation {
                HirAssertionMessageEvaluation::Default => lower_concrete_string(
                    context,
                    statements,
                    "assertion failed",
                    "assertion.default_message",
                ),
                HirAssertionMessageEvaluation::Folded => {
                    let HirExpressionKind::VariantConstruct {
                        carrier: HirVariantCarrier::Option,
                        variant_index: 1,
                        fields,
                    } = &context
                        .module_context
                        .hir_module
                        .expressions
                        .expression(*message)
                        .kind
                    else {
                        return Err(CompilerError::compiler_error(
                            "Wasm lowering received a non-option folded assertion message",
                        ));
                    };
                    let [field] = context
                        .module_context
                        .hir_module
                        .expressions
                        .variant_fields(*fields)
                    else {
                        return Err(CompilerError::compiler_error(
                            "Wasm lowering received a folded assertion message without one payload",
                        ));
                    };
                    lower_expression(context, field.value, statements)?
                }
                HirAssertionMessageEvaluation::Runtime => {
                    return Err(CompilerError::compiler_error(
                        "Wasm lowering received a runtime assertion message after target validation",
                    ));
                }
            };
            let import =
                ensure_host_import(context.module_context, WasmHostFunction::AssertionFailed);
            statements.push(WasmLirStmt::Call {
                dst: None,
                callee: WasmCalleeRef::Import(import),
                args: vec![message.value],
            });

            // A returning host must still end this unrecoverable invocation.
            Ok(WasmLirTerminator::Trap)
        }
        HirTerminator::Match { .. } => Err(lir_transformation_error(
            "Wasm lowering does not yet support HirTerminator::Match",
        )),
    }
}

fn lower_jump_argument_transfer(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    target: BlockId,
    args: &[HirJumpArgument],
    statements: &mut Vec<WasmLirStmt>,
) -> Result<crate::backends::wasm::lir::types::WasmLirBlockId, CompilerError> {
    let target_block = context
        .module_context
        .hir_module
        .blocks
        .iter()
        .find(|block| block.id == target)
        .ok_or_else(|| {
            lir_transformation_error(format!(
                "Wasm lowering could not resolve block id {target:?}"
            ))
        })?;
    let lir_target = resolve_block_id(context, target)?;

    if args.is_empty() {
        return Ok(lir_target);
    }

    let mut transfers = Vec::with_capacity(args.len());
    let mut destinations = HashSet::with_capacity(args.len());
    for argument in args {
        if !target_block
            .locals
            .iter()
            .any(|local| local.id == argument.destination)
        {
            return Err(lir_transformation_error(format!(
                "Wasm jump argument destination {:?} is not declared by target block {}",
                argument.destination, target.0
            )));
        }
        if !destinations.insert(argument.destination) {
            return Err(lir_transformation_error(format!(
                "Wasm jump to block {} defines destination {:?} more than once",
                target.0, argument.destination
            )));
        }

        let source = context
            .local_map
            .get(&argument.source)
            .copied()
            .ok_or_else(|| {
                lir_transformation_error(format!(
                    "Wasm lowering could not resolve jump source local {:?}",
                    argument.source
                ))
            })?;
        let destination = context
            .local_map
            .get(&argument.destination)
            .copied()
            .ok_or_else(|| {
                lir_transformation_error(format!(
                    "Wasm lowering could not resolve jump target local {:?}",
                    argument.destination
                ))
            })?;
        let source_abi = context
            .local_type_by_id
            .get(&source)
            .copied()
            .ok_or_else(|| {
                lir_transformation_error(format!(
                    "Wasm lowering could not resolve carrier ABI for jump source local {:?}",
                    argument.source
                ))
            })?;
        let destination_abi = context
            .local_type_by_id
            .get(&destination)
            .copied()
            .ok_or_else(|| {
                lir_transformation_error(format!(
                    "Wasm lowering could not resolve carrier ABI for jump target local {:?}",
                    argument.destination
                ))
            })?;

        if source_abi != destination_abi {
            return Err(lir_transformation_error(format!(
                "Wasm lowering cannot transfer jump argument from {:?} ({source_abi:?}) into target local {:?} ({destination_abi:?}) because their carrier ABI types differ",
                argument.source, argument.destination
            )));
        }

        transfers.push((source, destination, source_abi));
    }

    // Snapshot only sources that an earlier destination would overwrite. Common
    // one-argument merges need just one copy; cyclic transfers preserve old values.
    let mut captured = Vec::new();
    for (index, (source, _, abi)) in transfers.iter().enumerate() {
        if transfers[..index]
            .iter()
            .any(|(prior_source, destination, _)| {
                prior_source != destination && destination == source
            })
            && !captured.iter().any(|(original, _)| original == source)
        {
            let snapshot = context.alloc_temp(*abi);
            statements.push(WasmLirStmt::Copy {
                dst: snapshot,
                src: *source,
            });
            captured.push((*source, snapshot));
        }
    }

    for (source, destination, _) in transfers {
        if source != destination {
            let source = captured
                .iter()
                .find_map(|(original, snapshot)| (*original == source).then_some(*snapshot))
                .unwrap_or(source);
            statements.push(WasmLirStmt::Copy {
                dst: destination,
                src: source,
            });
        }
    }
    Ok(lir_target)
}

fn resolve_block_id(
    context: &WasmFunctionLoweringContext<'_, '_>,
    block_id: BlockId,
) -> Result<crate::backends::wasm::lir::types::WasmLirBlockId, CompilerError> {
    context.block_map.get(&block_id).copied().ok_or_else(|| {
        lir_transformation_error(format!(
            "Wasm lowering could not resolve block id {block_id:?}"
        ))
    })
}
