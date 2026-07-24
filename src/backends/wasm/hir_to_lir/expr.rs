//! Expression lowering helpers for HIR -> Wasm LIR.

use crate::backends::error_types::lir_transformation_error;
use crate::backends::wasm::hir_to_lir::context::{WasmFunctionLoweringContext, lower_type_to_abi};
use crate::backends::wasm::hir_to_lir::static_data::intern_static_utf8;
use crate::backends::wasm::lir::instructions::WasmLirStmt;
use crate::backends::wasm::lir::types::{WasmAbiType, WasmLirLocalId, WasmLocalRole};
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::operators::HirBinOp;
use crate::compiler_frontend::hir::places::HirPlace;

/// Result of lowering a single HIR expression into LIR statements and a destination local.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ExprLoweringOutput {
    /// Local containing the lowered expression value/handle.
    pub value: WasmLirLocalId,
    /// Advisory move hint for assignment/call-site lowering.
    /// WHY: LIR keeps move/copy distinct so ownership-aware lowering can evolve independently.
    pub prefer_move: bool,
}

pub(crate) fn lower_expression(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    expression: &HirExpression,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<ExprLoweringOutput, CompilerError> {
    // This matcher is intentionally partial while Wasm support is experimental. Unsupported HIR
    // expression kinds return structured LIR transformation errors instead of panicking.
    match &expression.kind {
        HirExpressionKind::Int(value) => {
            let dst = context.alloc_temp(WasmAbiType::I64);
            statements.push(WasmLirStmt::ConstI64 {
                dst,
                value: *value as i64,
            });
            Ok(ExprLoweringOutput {
                value: dst,
                prefer_move: false,
            })
        }
        HirExpressionKind::VariantConstruct {
            carrier,
            variant_index,
            fields,
        } => {
            if !fields.is_empty() {
                return Err(lir_transformation_error(
                    "Wasm backend does not yet support variant payload fields",
                ));
            }
            match carrier {
                crate::compiler_frontend::hir::expressions::HirVariantCarrier::Choice {
                    ..
                }
                | crate::compiler_frontend::hir::expressions::HirVariantCarrier::Option => {
                    let dst = context.alloc_temp(WasmAbiType::I64);
                    statements.push(WasmLirStmt::ConstI64 {
                        dst,
                        value: *variant_index as i64,
                    });
                    Ok(ExprLoweringOutput {
                        value: dst,
                        prefer_move: false,
                    })
                }
                #[cfg(test)]
                crate::compiler_frontend::hir::expressions::HirVariantCarrier::Fallible => {
                    let dst = context.alloc_temp(WasmAbiType::I64);
                    statements.push(WasmLirStmt::ConstI64 {
                        dst,
                        value: *variant_index as i64,
                    });
                    Ok(ExprLoweringOutput {
                        value: dst,
                        prefer_move: false,
                    })
                }
            }
        }
        // Moth `Float` is finite f64. HIR validation rejects non-finite literals before
        // any backend sees them, so this path trusts the invariant without rechecking.
        HirExpressionKind::Float(value) => {
            let dst = context.alloc_temp(WasmAbiType::F64);
            statements.push(WasmLirStmt::ConstF64 { dst, value: *value });
            Ok(ExprLoweringOutput {
                value: dst,
                prefer_move: false,
            })
        }
        HirExpressionKind::Bool(value) => {
            let dst = context.alloc_temp(WasmAbiType::I32);
            statements.push(WasmLirStmt::ConstI32 {
                dst,
                value: if *value { 1 } else { 0 },
            });
            Ok(ExprLoweringOutput {
                value: dst,
                prefer_move: false,
            })
        }
        HirExpressionKind::Char(value) => {
            let dst = context.alloc_temp(WasmAbiType::I32);
            statements.push(WasmLirStmt::ConstI32 {
                dst,
                value: *value as i32,
            });
            Ok(ExprLoweringOutput {
                value: dst,
                prefer_move: false,
            })
        }
        HirExpressionKind::StringLiteral(value) => {
            // String literal lowering goes through runtime buffer ops so the
            // same model can be reused for runtime template fragments.
            let data_id = intern_static_utf8(context.module_context, value, "hir.string_literal");
            let buffer =
                context.alloc_local(None, WasmAbiType::Handle, WasmLocalRole::BufferHandle);
            statements.push(WasmLirStmt::StringNewBuffer { dst: buffer });
            statements.push(WasmLirStmt::StringPushLiteral {
                buffer,
                data: data_id,
            });

            let dst = context.alloc_local(None, WasmAbiType::Handle, WasmLocalRole::ValueHandle);
            statements.push(WasmLirStmt::StringFinish { dst, buffer });

            Ok(ExprLoweringOutput {
                value: dst,
                prefer_move: false,
            })
        }
        HirExpressionKind::Load(place) => {
            let local = lower_place_local(context, place)?;
            Ok(ExprLoweringOutput {
                value: local,
                prefer_move: true,
            })
        }
        HirExpressionKind::Copy(place) => {
            let local = lower_place_local(context, place)?;
            Ok(ExprLoweringOutput {
                value: local,
                prefer_move: false,
            })
        }
        HirExpressionKind::BinOp { left, op, right } => {
            lower_binary_expression(context, expression, left, *op, right, statements)
        }
        HirExpressionKind::UnaryOp { op, .. } => Err(lir_transformation_error(format!(
            "Wasm lowering does not yet support unary operator {op:?}"
        ))),
        HirExpressionKind::Collection(items) => {
            if is_empty_string_collection(context, expression, items) {
                let dst =
                    context.alloc_local(None, WasmAbiType::Handle, WasmLocalRole::ValueHandle);
                statements.push(WasmLirStmt::VecNew { dst });
                return Ok(ExprLoweringOutput {
                    value: dst,
                    prefer_move: false,
                });
            }

            Err(lir_transformation_error(
                "Wasm lowering only supports empty Vec<String> collection literals in this pass",
            ))
        }
        HirExpressionKind::MapLiteral(_) => Err(lir_transformation_error(
            "Wasm hashmap literal reached lowering before backend feature validation",
        )),
        HirExpressionKind::Cast { .. } => Err(lir_transformation_error(
            "Wasm lowering does not yet support cast expressions",
        )),
        HirExpressionKind::StructConstruct { .. }
        | HirExpressionKind::Range { .. }
        | HirExpressionKind::TupleConstruct { .. }
        | HirExpressionKind::TupleGet { .. }
        | HirExpressionKind::FallibleUnwrapSuccess { .. }
        | HirExpressionKind::FallibleUnwrapError { .. }
        | HirExpressionKind::VariantPayloadGet { .. } => Err(lir_transformation_error(
            "Wasm lowering does not yet support this HIR expression",
        )),
    }
}

fn lower_binary_expression(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    expression: &HirExpression,
    left: &HirExpression,
    op: HirBinOp,
    right: &HirExpression,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<ExprLoweringOutput, CompilerError> {
    if matches!(op, HirBinOp::Add) && should_lower_as_string_concat(context, expression) {
        return lower_string_concat_expression(context, expression, statements);
    }

    let lhs_abi = expression_abi(context, left);
    let rhs_abi = expression_abi(context, right);
    let lhs = lower_expression(context, left, statements)?;
    let rhs = lower_expression(context, right, statements)?;

    match op {
        HirBinOp::Eq => {
            let dst = context.alloc_temp(WasmAbiType::I32);
            statements.push(WasmLirStmt::IntEq {
                dst,
                lhs: lhs.value,
                rhs: rhs.value,
            });
            Ok(ExprLoweringOutput {
                value: dst,
                prefer_move: false,
            })
        }
        HirBinOp::Ne => {
            let dst = context.alloc_temp(WasmAbiType::I32);
            statements.push(WasmLirStmt::IntNe {
                dst,
                lhs: lhs.value,
                rhs: rhs.value,
            });
            Ok(ExprLoweringOutput {
                value: dst,
                prefer_move: false,
            })
        }
        HirBinOp::Add => {
            if lhs_abi != rhs_abi {
                return Err(lir_transformation_error(format!(
                    "Wasm lowering does not support Add for mismatched ABI types {lhs_abi:?} and {rhs_abi:?}"
                )));
            }

            match lhs_abi {
                WasmAbiType::I64 => {
                    let dst = context.alloc_temp(WasmAbiType::I64);
                    statements.push(WasmLirStmt::IntAdd {
                        dst,
                        lhs: lhs.value,
                        rhs: rhs.value,
                    });
                    Ok(ExprLoweringOutput {
                        value: dst,
                        prefer_move: false,
                    })
                }
                WasmAbiType::F32 | WasmAbiType::F64 => {
                    let dst = context.alloc_temp(lhs_abi);
                    statements.push(WasmLirStmt::FloatAdd {
                        dst,
                        lhs: lhs.value,
                        rhs: rhs.value,
                    });
                    Ok(ExprLoweringOutput {
                        value: dst,
                        prefer_move: false,
                    })
                }
                _ => Err(lir_transformation_error(format!(
                    "Wasm lowering does not support Add for ABI type {lhs_abi:?}"
                ))),
            }
        }
        HirBinOp::Sub => {
            if lhs_abi != rhs_abi {
                return Err(lir_transformation_error(format!(
                    "Wasm lowering does not support Sub for mismatched ABI types {lhs_abi:?} and {rhs_abi:?}"
                )));
            }

            match lhs_abi {
                WasmAbiType::I64 => {
                    let dst = context.alloc_temp(WasmAbiType::I64);
                    statements.push(WasmLirStmt::IntSub {
                        dst,
                        lhs: lhs.value,
                        rhs: rhs.value,
                    });
                    Ok(ExprLoweringOutput {
                        value: dst,
                        prefer_move: false,
                    })
                }
                WasmAbiType::F32 | WasmAbiType::F64 => {
                    let dst = context.alloc_temp(lhs_abi);
                    statements.push(WasmLirStmt::FloatSub {
                        dst,
                        lhs: lhs.value,
                        rhs: rhs.value,
                    });
                    Ok(ExprLoweringOutput {
                        value: dst,
                        prefer_move: false,
                    })
                }
                _ => Err(lir_transformation_error(format!(
                    "Wasm lowering does not support Sub for ABI type {lhs_abi:?}"
                ))),
            }
        }
        HirBinOp::Lt | HirBinOp::Le | HirBinOp::Gt | HirBinOp::Ge => {
            if lhs_abi != rhs_abi {
                return Err(lir_transformation_error(format!(
                    "Wasm lowering does not support ordered comparison {op:?} for mismatched ABI types {lhs_abi:?} and {rhs_abi:?}"
                )));
            }

            match lhs_abi {
                WasmAbiType::I32 | WasmAbiType::I64 | WasmAbiType::F32 | WasmAbiType::F64 => {
                    let dst = context.alloc_temp(WasmAbiType::I32);
                    let statement = match op {
                        HirBinOp::Lt => WasmLirStmt::OrderedLt {
                            dst,
                            lhs: lhs.value,
                            rhs: rhs.value,
                        },
                        HirBinOp::Le => WasmLirStmt::OrderedLe {
                            dst,
                            lhs: lhs.value,
                            rhs: rhs.value,
                        },
                        HirBinOp::Gt => WasmLirStmt::OrderedGt {
                            dst,
                            lhs: lhs.value,
                            rhs: rhs.value,
                        },
                        HirBinOp::Ge => WasmLirStmt::OrderedGe {
                            dst,
                            lhs: lhs.value,
                            rhs: rhs.value,
                        },
                        _ => unreachable!("ordered branch already filtered non-ordered operators"),
                    };
                    statements.push(statement);
                    Ok(ExprLoweringOutput {
                        value: dst,
                        prefer_move: false,
                    })
                }
                _ => Err(lir_transformation_error(format!(
                    "Wasm lowering does not support ordered comparison {op:?} for ABI type {lhs_abi:?}"
                ))),
            }
        }
        HirBinOp::Mod => {
            if lhs_abi != rhs_abi {
                return Err(binop_abi_mismatch_error("Mod", lhs_abi, rhs_abi));
            }
            match lhs_abi {
                WasmAbiType::I64 => {
                    let dst = context.alloc_temp(WasmAbiType::I64);
                    statements.push(WasmLirStmt::IntMod {
                        dst,
                        lhs: lhs.value,
                        rhs: rhs.value,
                    });
                    Ok(ExprLoweringOutput {
                        value: dst,
                        prefer_move: false,
                    })
                }
                WasmAbiType::F32 | WasmAbiType::F64 => {
                    let dst = context.alloc_temp(lhs_abi);
                    statements.push(WasmLirStmt::FloatMod {
                        dst,
                        lhs: lhs.value,
                        rhs: rhs.value,
                    });
                    Ok(ExprLoweringOutput {
                        value: dst,
                        prefer_move: false,
                    })
                }
                _ => Err(binop_unsupported_abi_error("Mod", lhs_abi)),
            }
        }
        HirBinOp::Mul => {
            if lhs_abi != rhs_abi {
                return Err(binop_abi_mismatch_error("Mul", lhs_abi, rhs_abi));
            }
            match lhs_abi {
                WasmAbiType::I64 => {
                    let dst = context.alloc_temp(WasmAbiType::I64);
                    statements.push(WasmLirStmt::IntMul {
                        dst,
                        lhs: lhs.value,
                        rhs: rhs.value,
                    });
                    Ok(ExprLoweringOutput {
                        value: dst,
                        prefer_move: false,
                    })
                }
                WasmAbiType::F32 | WasmAbiType::F64 => {
                    let dst = context.alloc_temp(lhs_abi);
                    statements.push(WasmLirStmt::FloatMul {
                        dst,
                        lhs: lhs.value,
                        rhs: rhs.value,
                    });
                    Ok(ExprLoweringOutput {
                        value: dst,
                        prefer_move: false,
                    })
                }
                _ => Err(binop_unsupported_abi_error("Mul", lhs_abi)),
            }
        }
        HirBinOp::Div => {
            match lhs_abi {
                WasmAbiType::I64 => {
                    // Int / Int → Float: type system guarantees Float result even with Int operands.
                    let dst = context.alloc_temp(WasmAbiType::F64);
                    statements.push(WasmLirStmt::IntToFloatDiv {
                        dst,
                        lhs: lhs.value,
                        rhs: rhs.value,
                    });
                    Ok(ExprLoweringOutput {
                        value: dst,
                        prefer_move: false,
                    })
                }
                WasmAbiType::F32 | WasmAbiType::F64 => {
                    if lhs_abi != rhs_abi {
                        return Err(binop_abi_mismatch_error("Div", lhs_abi, rhs_abi));
                    }
                    let dst = context.alloc_temp(lhs_abi);
                    statements.push(WasmLirStmt::FloatDiv {
                        dst,
                        lhs: lhs.value,
                        rhs: rhs.value,
                    });
                    Ok(ExprLoweringOutput {
                        value: dst,
                        prefer_move: false,
                    })
                }
                _ => Err(binop_unsupported_abi_error("Div", lhs_abi)),
            }
        }
        HirBinOp::IntDiv => {
            if lhs_abi != rhs_abi {
                return Err(binop_abi_mismatch_error("IntDiv", lhs_abi, rhs_abi));
            }
            match lhs_abi {
                WasmAbiType::I64 => {
                    let dst = context.alloc_temp(WasmAbiType::I64);
                    statements.push(WasmLirStmt::IntFloorDiv {
                        dst,
                        lhs: lhs.value,
                        rhs: rhs.value,
                    });
                    Ok(ExprLoweringOutput {
                        value: dst,
                        prefer_move: false,
                    })
                }
                _ => Err(binop_unsupported_abi_error("IntDiv", lhs_abi)),
            }
        }
        HirBinOp::And => match lhs_abi {
            WasmAbiType::I32 => {
                let dst = context.alloc_temp(WasmAbiType::I32);
                statements.push(WasmLirStmt::BoolAnd {
                    dst,
                    lhs: lhs.value,
                    rhs: rhs.value,
                });
                Ok(ExprLoweringOutput {
                    value: dst,
                    prefer_move: false,
                })
            }
            _ => Err(binop_unsupported_abi_error("And", lhs_abi)),
        },
        HirBinOp::Or => match lhs_abi {
            WasmAbiType::I32 => {
                let dst = context.alloc_temp(WasmAbiType::I32);
                statements.push(WasmLirStmt::BoolOr {
                    dst,
                    lhs: lhs.value,
                    rhs: rhs.value,
                });
                Ok(ExprLoweringOutput {
                    value: dst,
                    prefer_move: false,
                })
            }
            _ => Err(binop_unsupported_abi_error("Or", lhs_abi)),
        },
        _ => Err(lir_transformation_error(format!(
            "Wasm lowering does not yet support binary operator {op:?}"
        ))),
    }
}

fn binop_abi_mismatch_error(op: &str, lhs: WasmAbiType, rhs: WasmAbiType) -> CompilerError {
    lir_transformation_error(format!(
        "Wasm lowering does not support {op} for mismatched ABI types {lhs:?} and {rhs:?}"
    ))
}

fn binop_unsupported_abi_error(op: &str, abi: WasmAbiType) -> CompilerError {
    lir_transformation_error(format!(
        "Wasm lowering does not support {op} for ABI type {abi:?}"
    ))
}

fn should_lower_as_string_concat(
    context: &WasmFunctionLoweringContext<'_, '_>,
    expression: &HirExpression,
) -> bool {
    is_handle_type(context, expression)
}

fn lower_string_concat_expression(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    expression: &HirExpression,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<ExprLoweringOutput, CompilerError> {
    // WHAT: lower string `Add` chains into explicit buffer operations.
    // WHY: both normal functions and runtime fragments should follow the same string-concat path
    // so control-flow-heavy runtime wrappers do not need a second lowering contract.
    let mut chunks = Vec::new();
    collect_string_concat_chunks(context, expression, &mut chunks);

    let buffer = context.alloc_local(None, WasmAbiType::Handle, WasmLocalRole::BufferHandle);
    statements.push(WasmLirStmt::StringNewBuffer { dst: buffer });

    for chunk in chunks {
        match &chunk.kind {
            HirExpressionKind::StringLiteral(literal) => {
                let static_id =
                    intern_static_utf8(context.module_context, literal, "hir.string_concat");
                statements.push(WasmLirStmt::StringPushLiteral {
                    buffer,
                    data: static_id,
                });
            }
            _ => {
                let lowered = lower_expression(context, chunk, statements)?;
                let chunk_handle = match expression_abi(context, chunk) {
                    WasmAbiType::Handle => lowered.value,
                    WasmAbiType::I64 => {
                        let converted = context.alloc_local(
                            None,
                            WasmAbiType::Handle,
                            WasmLocalRole::ValueHandle,
                        );
                        statements.push(WasmLirStmt::StringFromI64 {
                            dst: converted,
                            value: lowered.value,
                        });
                        converted
                    }
                    other => {
                        return Err(lir_transformation_error(format!(
                            "Wasm lowering string concatenation requires handle-compatible chunks, found {other:?}"
                        )));
                    }
                };
                statements.push(WasmLirStmt::StringPushHandle {
                    buffer,
                    handle: chunk_handle,
                });
            }
        }
    }

    let dst = context.alloc_local(None, WasmAbiType::Handle, WasmLocalRole::ValueHandle);
    statements.push(WasmLirStmt::StringFinish { dst, buffer });

    Ok(ExprLoweringOutput {
        value: dst,
        prefer_move: false,
    })
}

fn collect_string_concat_chunks<'a>(
    context: &WasmFunctionLoweringContext<'_, '_>,
    expression: &'a HirExpression,
    out: &mut Vec<&'a HirExpression>,
) {
    if let HirExpressionKind::BinOp { left, op, right } = &expression.kind
        && matches!(op, HirBinOp::Add)
        && is_handle_type(context, expression)
    {
        collect_string_concat_chunks(context, left, out);
        collect_string_concat_chunks(context, right, out);
        return;
    }

    out.push(expression);
}

fn is_handle_type(
    context: &WasmFunctionLoweringContext<'_, '_>,
    expression: &HirExpression,
) -> bool {
    matches!(expression_abi(context, expression), WasmAbiType::Handle)
}

fn is_empty_string_collection(
    context: &WasmFunctionLoweringContext<'_, '_>,
    expression: &HirExpression,
    items: &[HirExpression],
) -> bool {
    if !items.is_empty() {
        return false;
    }

    let Some(element) = context
        .module_context
        .type_environment
        .collection_element_type(expression.ty)
    else {
        return false;
    };

    element == context.module_context.type_environment.builtins().string
}

fn expression_abi(
    context: &WasmFunctionLoweringContext<'_, '_>,
    expression: &HirExpression,
) -> WasmAbiType {
    lower_type_to_abi(context.module_context, expression.ty)
}

fn lower_place_local(
    context: &WasmFunctionLoweringContext<'_, '_>,
    place: &HirPlace,
) -> Result<WasmLirLocalId, CompilerError> {
    // WHAT: place lowering currently supports direct locals only.
    // WHY: field/index projections require additional memory-model and layout work.
    match place {
        HirPlace::Local(local_id) => context.local_map.get(local_id).copied().ok_or_else(|| {
            lir_transformation_error(format!(
                "Wasm lowering could not resolve local {local_id:?}",
            ))
        }),
        HirPlace::Field { .. } | HirPlace::Index { .. } => Err(lir_transformation_error(
            "Wasm lowering currently supports only direct local places",
        )),
    }
}
