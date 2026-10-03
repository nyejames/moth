//! Expression lowering helpers for HIR -> Wasm LIR.

use crate::backends::error_types::lir_transformation_error;
use crate::backends::wasm::hir_to_lir::context::{WasmFunctionLoweringContext, lower_type_to_abi};
use crate::backends::wasm::hir_to_lir::static_data::intern_static_utf8;
use crate::backends::wasm::lir::instructions::{
    WasmLirStmt, WasmScalarComparisonOp, WasmScalarComparisonType,
};
use crate::backends::wasm::lir::types::{WasmAbiType, WasmLirLocalId, WasmLocalRole};
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::builtins::casts::evidence::numeric_conversion_fallibility;
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId,
};
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_operators::comparison_supported;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::operators::HirBinOp;
use crate::compiler_frontend::hir::places::HirPlace;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass};
use moth_lexical::numeric::precision::BinaryFloatPrecision;
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth};
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
            let abi = lower_type_to_abi(context.module_context, expression.ty);
            let dst = context.alloc_temp(abi);
            match abi {
                WasmAbiType::I32 => {
                    let value = i32::try_from(*value).map_err(|_| {
                        lir_transformation_error(format!(
                            "Wasm lowering received Int value {value} outside the selected I32 profile"
                        ))
                    })?;
                    statements.push(WasmLirStmt::ConstI32 { dst, value });
                }
                WasmAbiType::I64 => statements.push(WasmLirStmt::ConstI64 { dst, value: *value }),
                other => {
                    return Err(lir_transformation_error(format!(
                        "Wasm lowering expected an integer carrier for Int, found {other:?}"
                    )));
                }
            }
            Ok(ExprLoweringOutput {
                value: dst,
                prefer_move: false,
            })
        }
        HirExpressionKind::FixedScalar(value) => {
            let scalar = value.scalar();
            let abi = lower_type_to_abi(context.module_context, expression.ty);
            let dst = context.alloc_temp(abi);
            match scalar {
                FixedScalar::I8 | FixedScalar::I16 | FixedScalar::I32 => {
                    let value = value.as_i64().ok_or_else(|| {
                        lir_transformation_error(format!(
                            "Wasm lowering received an invalid {scalar:?} integer value"
                        ))
                    })?;
                    let value = i32::try_from(value).map_err(|_| {
                        lir_transformation_error(format!(
                            "Wasm lowering received out-of-range {scalar:?} integer value"
                        ))
                    })?;
                    statements.push(WasmLirStmt::ConstI32 { dst, value });
                }
                FixedScalar::U8 | FixedScalar::U16 | FixedScalar::U32 | FixedScalar::Byte => {
                    let value = value.as_u64().ok_or_else(|| {
                        lir_transformation_error(format!(
                            "Wasm lowering received an invalid {scalar:?} integer value"
                        ))
                    })?;
                    let value = u32::try_from(value).map_err(|_| {
                        lir_transformation_error(format!(
                            "Wasm lowering received out-of-range {scalar:?} integer value"
                        ))
                    })? as i32;
                    statements.push(WasmLirStmt::ConstI32 { dst, value });
                }
                FixedScalar::I64 => {
                    let value = value.as_i64().ok_or_else(|| {
                        lir_transformation_error("Wasm lowering received an invalid I64 value")
                    })?;
                    statements.push(WasmLirStmt::ConstI64 { dst, value });
                }
                FixedScalar::U64 => {
                    let value = value.as_u64().ok_or_else(|| {
                        lir_transformation_error("Wasm lowering received an invalid U64 value")
                    })?;
                    statements.push(WasmLirStmt::ConstI64 {
                        dst,
                        value: value as i64,
                    });
                }
                FixedScalar::F32 => {
                    let value = value.as_f64().ok_or_else(|| {
                        lir_transformation_error("Wasm lowering received an invalid F32 value")
                    })? as f32;
                    statements.push(WasmLirStmt::ConstF32 { dst, value });
                }
                FixedScalar::F64 => {
                    let value = value.as_f64().ok_or_else(|| {
                        lir_transformation_error("Wasm lowering received an invalid F64 value")
                    })?;
                    statements.push(WasmLirStmt::ConstF64 { dst, value });
                }
                FixedScalar::F16 => {
                    let value = value.as_f64().ok_or_else(|| {
                        lir_transformation_error("Wasm lowering received an invalid F16 value")
                    })?;
                    // Every canonical binary16 value is exact in F32, including signed zero.
                    statements.push(WasmLirStmt::ConstF32 {
                        dst,
                        value: value as f32,
                    });
                }
            }
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
        // HIR has already rounded `Float` to the boundary profile precision.
        HirExpressionKind::Float(value) => {
            let abi = lower_type_to_abi(context.module_context, expression.ty);
            let dst = context.alloc_temp(abi);
            match abi {
                WasmAbiType::F32 => statements.push(WasmLirStmt::ConstF32 {
                    dst,
                    value: *value as f32,
                }),
                WasmAbiType::F64 => statements.push(WasmLirStmt::ConstF64 { dst, value: *value }),
                other => {
                    return Err(lir_transformation_error(format!(
                        "Wasm lowering expected a float carrier for Float, found {other:?}"
                    )));
                }
            }
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
            // String literal lowering goes through runtime buffer ops so the same model can be
            // reused for runtime template fragments.
            Ok(lower_concrete_string(
                context,
                statements,
                value,
                "hir.string_literal",
            ))
        }
        HirExpressionKind::StructuralString { pieces } => {
            let rendered = render_structural_string(context, pieces)?;
            Ok(lower_concrete_string(
                context,
                statements,
                &rendered,
                "hir.structural_string",
            ))
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
        HirExpressionKind::Cast { source, policy } => match *policy {
            BuiltinCastPolicyId::NumericConversion {
                source: source_domain,
                target: target_domain,
            } => lower_infallible_numeric_conversion(
                context,
                source_domain,
                target_domain,
                source,
                expression,
                statements,
            ),
            BuiltinCastPolicyId::NumericToString(source_domain) => {
                lower_infallible_numeric_to_string(
                    context,
                    source_domain,
                    source,
                    expression,
                    statements,
                )
            }
            BuiltinCastPolicyId::ByteToU8 => lower_infallible_byte_conversion(
                context,
                FixedScalar::Byte,
                FixedScalar::U8,
                source,
                expression,
                statements,
            ),
            BuiltinCastPolicyId::U8ToByte => lower_infallible_byte_conversion(
                context,
                FixedScalar::U8,
                FixedScalar::Byte,
                source,
                expression,
                statements,
            ),
            _ => Err(lir_transformation_error(format!(
                "Wasm lowering does not yet support cast policy {policy:?}"
            ))),
        },
        HirExpressionKind::Number(_) => Err(lir_transformation_error(
            "Wasm lowering reached a Dec literal after target validation",
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

fn lower_infallible_numeric_conversion(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    source_domain: NumericScalar,
    target_domain: NumericScalar,
    source_expression: &HirExpression,
    target_expression: &HirExpression,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<ExprLoweringOutput, CompilerError> {
    if matches!(source_domain, NumericScalar::Number(_))
        || matches!(target_domain, NumericScalar::Number(_))
    {
        return Err(lir_transformation_error(
            "Wasm lowering does not support Dec conversion casts",
        ));
    }
    let profile = context.module_context.request.numeric_profile;
    let type_environment = context.module_context.type_environment;
    if source_domain == target_domain
        || source_expression.ty != source_domain.type_id(type_environment)
        || target_expression.ty != target_domain.type_id(type_environment)
        || numeric_conversion_fallibility(source_domain, target_domain, profile)
            != BuiltinCastFallibility::Infallible
    {
        return Err(lir_transformation_error(
            "Wasm numeric conversion has inconsistent or fallible HIR cast evidence",
        ));
    }

    let source_precision = source_domain.binary_float_precision(profile);
    let target_precision = target_domain.binary_float_precision(profile);

    let source_abi = expression_abi(context, source_expression);
    let target_abi = expression_abi(context, target_expression);
    if source_abi == target_abi {
        let source_value = lower_expression(context, source_expression, statements)?;
        return Ok(ExprLoweringOutput {
            value: source_value.value,
            prefer_move: false,
        });
    }

    if source_domain.is_integer() && target_domain.is_integer() {
        let Some((source_minimum, _)) = source_domain.integer_range(profile) else {
            return Err(lir_transformation_error(
                "Wasm integer conversion source has no canonical integer range",
            ));
        };
        if source_abi == WasmAbiType::I32 && target_abi == WasmAbiType::I64 {
            let source_value = lower_expression(context, source_expression, statements)?;
            let dst = context.alloc_temp(WasmAbiType::I64);
            statements.push(WasmLirStmt::IntegerExtend {
                dst,
                source: source_value.value,
                source_signed: source_minimum < 0,
            });
            return Ok(ExprLoweringOutput {
                value: dst,
                prefer_move: false,
            });
        }
    }

    if source_domain.is_integer() {
        let Some((source_minimum, _)) = source_domain.integer_range(profile) else {
            return Err(lir_transformation_error(
                "Wasm integer conversion source has no canonical integer range",
            ));
        };
        if matches!(source_abi, WasmAbiType::I32 | WasmAbiType::I64)
            && matches!(
                (target_precision, target_abi),
                (
                    Some(BinaryFloatPrecision::Binary16 | BinaryFloatPrecision::Binary32),
                    WasmAbiType::F32
                ) | (Some(BinaryFloatPrecision::Binary64), WasmAbiType::F64)
            )
        {
            let source_value = lower_expression(context, source_expression, statements)?;
            let float_value = context.alloc_temp(target_abi);
            statements.push(WasmLirStmt::IntegerToFloat {
                dst: float_value,
                source: source_value.value,
                source_signed: source_minimum < 0,
            });

            if target_precision == Some(BinaryFloatPrecision::Binary16) {
                let dst = context.alloc_temp(WasmAbiType::F32);
                statements.push(WasmLirStmt::RoundF16 {
                    dst,
                    source: float_value,
                });
                return Ok(ExprLoweringOutput {
                    value: dst,
                    prefer_move: false,
                });
            }

            return Ok(ExprLoweringOutput {
                value: float_value,
                prefer_move: false,
            });
        }
    }

    if matches!(
        (source_precision, target_precision, source_abi, target_abi),
        (
            Some(BinaryFloatPrecision::Binary16 | BinaryFloatPrecision::Binary32),
            Some(BinaryFloatPrecision::Binary64),
            WasmAbiType::F32,
            WasmAbiType::F64
        )
    ) {
        let source_value = lower_expression(context, source_expression, statements)?;
        let dst = context.alloc_temp(WasmAbiType::F64);
        statements.push(WasmLirStmt::FloatExtend {
            dst,
            source: source_value.value,
        });
        return Ok(ExprLoweringOutput {
            value: dst,
            prefer_move: false,
        });
    }

    Err(lir_transformation_error(format!(
        "Wasm numeric conversion cannot map {source_domain} ({source_abi:?}) to {target_domain} ({target_abi:?})"
    )))
}

fn lower_infallible_numeric_to_string(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    source_domain: NumericScalar,
    source_expression: &HirExpression,
    target_expression: &HirExpression,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<ExprLoweringOutput, CompilerError> {
    if matches!(source_domain, NumericScalar::Number(_)) {
        return Err(lir_transformation_error(
            "Wasm lowering does not support Dec-to-String casts",
        ));
    }

    let type_environment = context.module_context.type_environment;
    let profile = context.module_context.request.numeric_profile;
    let float_precision = match source_domain {
        NumericScalar::Fixed(FixedScalar::F16 | FixedScalar::F32 | FixedScalar::F64) => {
            source_domain.binary_float_precision(profile)
        }
        _ => None,
    };
    let source_minimum = source_domain
        .integer_range(profile)
        .map(|(minimum, _)| minimum);
    if (!source_domain.is_integer() && float_precision.is_none())
        || (source_domain.is_integer() && source_minimum.is_none())
        || source_expression.ty != source_domain.type_id(type_environment)
        || target_expression.ty != type_environment.builtins().string
    {
        return Err(lir_transformation_error(
            "Wasm NumericToString cast has inconsistent or unsupported HIR evidence",
        ));
    }

    let source_abi = expression_abi(context, source_expression);
    let expected_source_abi = match float_precision {
        Some(BinaryFloatPrecision::Binary16 | BinaryFloatPrecision::Binary32) => WasmAbiType::F32,
        Some(BinaryFloatPrecision::Binary64) => WasmAbiType::F64,
        None if source_domain.is_integer() => source_abi,
        None => {
            return Err(lir_transformation_error(
                "Wasm NumericToString source has no supported numeric precision",
            ));
        }
    };
    if source_abi != expected_source_abi
        || (float_precision.is_none() && !matches!(source_abi, WasmAbiType::I32 | WasmAbiType::I64))
        || expression_abi(context, target_expression) != WasmAbiType::Handle
    {
        return Err(lir_transformation_error(format!(
            "Wasm NumericToString cannot map {source_domain} ({source_abi:?}) to a string handle"
        )));
    }

    // Evaluate a cast source once; the LIR carries its canonical domain precision separately.
    let source_value = lower_expression(context, source_expression, statements)?;
    if context.local_type_by_id.get(&source_value.value).copied() != Some(source_abi) {
        return Err(lir_transformation_error(format!(
            "Wasm NumericToString source lowered to an unexpected carrier for {source_domain}"
        )));
    }
    let dst = context.alloc_local(None, WasmAbiType::Handle, WasmLocalRole::ValueHandle);
    let statement = if let Some(precision) = float_precision {
        WasmLirStmt::StringFromFloat {
            dst,
            value: source_value.value,
            precision,
        }
    } else if matches!(source_minimum, Some(minimum) if minimum < 0) {
        WasmLirStmt::StringFromI64 {
            dst,
            value: source_value.value,
        }
    } else {
        WasmLirStmt::StringFromU64 {
            dst,
            value: source_value.value,
        }
    };
    statements.push(statement);
    Ok(ExprLoweringOutput {
        value: dst,
        prefer_move: false,
    })
}

fn lower_infallible_byte_conversion(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    source_scalar: FixedScalar,
    target_scalar: FixedScalar,
    source_expression: &HirExpression,
    target_expression: &HirExpression,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<ExprLoweringOutput, CompilerError> {
    if source_expression.ty != builtin_type_ids::fixed_scalar(source_scalar)
        || target_expression.ty != builtin_type_ids::fixed_scalar(target_scalar)
        || !matches!(
            (source_scalar, target_scalar),
            (FixedScalar::Byte, FixedScalar::U8) | (FixedScalar::U8, FixedScalar::Byte)
        )
        || expression_abi(context, source_expression) != WasmAbiType::I32
        || expression_abi(context, target_expression) != WasmAbiType::I32
    {
        return Err(lir_transformation_error(
            "Wasm Byte/U8 conversion has inconsistent HIR types or carriers",
        ));
    }

    let source_value = lower_expression(context, source_expression, statements)?;
    Ok(ExprLoweringOutput {
        value: source_value.value,
        prefer_move: false,
    })
}

fn render_structural_string(
    context: &WasmFunctionLoweringContext<'_, '_>,
    pieces: &[ConstStringPiece],
) -> Result<String, CompilerError> {
    let Some(url_map) = context
        .module_context
        .request
        .structural_string_urls
        .as_ref()
    else {
        return Err(lir_transformation_error(
            "Wasm lowering received a structural string without a builder URL map",
        ));
    };

    let mut rendered = String::new();
    for piece in pieces {
        match piece {
            ConstStringPiece::Text(text) => {
                rendered.push_str(context.module_context.string_table.resolve(*text))
            }
            ConstStringPiece::Resource(resource_id) => {
                let Some(url) = url_map.resource_urls.get(resource_id) else {
                    return Err(lir_transformation_error(format!(
                        "Wasm lowering has no rendered URL for structural resource {resource_id:?}"
                    )));
                };
                rendered.push_str(url);
            }
            ConstStringPiece::SiteRoot => {
                let Some(url) = url_map.site_root_url.as_deref() else {
                    return Err(lir_transformation_error(
                        "Wasm lowering has no rendered URL for a structural site root",
                    ));
                };
                rendered.push_str(url);
            }
        }
    }

    Ok(rendered)
}

fn lower_concrete_string(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    statements: &mut Vec<WasmLirStmt>,
    value: &str,
    debug_name: &str,
) -> ExprLoweringOutput {
    let data_id = intern_static_utf8(context.module_context, value, debug_name);
    let buffer = context.alloc_local(None, WasmAbiType::Handle, WasmLocalRole::BufferHandle);
    statements.push(WasmLirStmt::StringNewBuffer { dst: buffer });
    statements.push(WasmLirStmt::StringPushLiteral {
        buffer,
        data: data_id,
    });

    let dst = context.alloc_local(None, WasmAbiType::Handle, WasmLocalRole::ValueHandle);
    statements.push(WasmLirStmt::StringFinish { dst, buffer });

    ExprLoweringOutput {
        value: dst,
        prefer_move: false,
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
    if matches!(op, HirBinOp::StringAppend) {
        return lower_string_concat_expression(context, expression, statements);
    }

    let lhs_abi = expression_abi(context, left);
    let rhs_abi = expression_abi(context, right);
    let is_string_equality = matches!(op, HirBinOp::Eq | HirBinOp::Ne)
        && left.ty == context.module_context.type_environment.builtins().string
        && right.ty == context.module_context.type_environment.builtins().string;
    let lhs = lower_expression(context, left, statements)?;
    let rhs = lower_expression(context, right, statements)?;

    match op {
        HirBinOp::Eq => {
            let dst = context.alloc_temp(WasmAbiType::I32);
            let scalar_types = scalar_comparison_types(context, left.ty, right.ty, HirBinOp::Eq)?;
            let statement = if is_string_equality {
                WasmLirStmt::StringEq {
                    dst,
                    lhs: lhs.value,
                    rhs: rhs.value,
                }
            } else if let Some((lhs_type, rhs_type)) = scalar_types {
                WasmLirStmt::ScalarCompare {
                    dst,
                    lhs: lhs.value,
                    rhs: rhs.value,
                    op: WasmScalarComparisonOp::Eq,
                    lhs_type,
                    rhs_type,
                }
            } else {
                WasmLirStmt::IntEq {
                    dst,
                    lhs: lhs.value,
                    rhs: rhs.value,
                }
            };
            statements.push(statement);
            Ok(ExprLoweringOutput {
                value: dst,
                prefer_move: false,
            })
        }
        HirBinOp::Ne => {
            let dst = context.alloc_temp(WasmAbiType::I32);
            let scalar_types = scalar_comparison_types(context, left.ty, right.ty, HirBinOp::Ne)?;
            let statement = if is_string_equality {
                WasmLirStmt::StringNe {
                    dst,
                    lhs: lhs.value,
                    rhs: rhs.value,
                }
            } else if let Some((lhs_type, rhs_type)) = scalar_types {
                WasmLirStmt::ScalarCompare {
                    dst,
                    lhs: lhs.value,
                    rhs: rhs.value,
                    op: WasmScalarComparisonOp::Ne,
                    lhs_type,
                    rhs_type,
                }
            } else {
                WasmLirStmt::IntNe {
                    dst,
                    lhs: lhs.value,
                    rhs: rhs.value,
                }
            };
            statements.push(statement);
            Ok(ExprLoweringOutput {
                value: dst,
                prefer_move: false,
            })
        }
        HirBinOp::Lt | HirBinOp::Le | HirBinOp::Gt | HirBinOp::Ge => {
            let scalar_types = scalar_comparison_types(context, left.ty, right.ty, op)?;
            let dst = context.alloc_temp(WasmAbiType::I32);
            if let Some((lhs_type, rhs_type)) = scalar_types {
                statements.push(WasmLirStmt::ScalarCompare {
                    dst,
                    lhs: lhs.value,
                    rhs: rhs.value,
                    op: scalar_comparison_op(op),
                    lhs_type,
                    rhs_type,
                });
                return Ok(ExprLoweringOutput {
                    value: dst,
                    prefer_move: false,
                });
            }

            if lhs_abi != rhs_abi {
                return Err(lir_transformation_error(format!(
                    "Wasm lowering does not support ordered comparison {op:?} for mismatched ABI types {lhs_abi:?} and {rhs_abi:?}"
                )));
            }

            match lhs_abi {
                WasmAbiType::I32 | WasmAbiType::I64 | WasmAbiType::F32 | WasmAbiType::F64 => {
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

fn scalar_comparison_types(
    context: &WasmFunctionLoweringContext<'_, '_>,
    lhs_type: crate::compiler_frontend::datatypes::ids::TypeId,
    rhs_type: crate::compiler_frontend::datatypes::ids::TypeId,
    op: HirBinOp,
) -> Result<Option<(WasmScalarComparisonType, WasmScalarComparisonType)>, CompilerError> {
    let environment = &context.module_context.type_environment;
    let lhs_numeric = NumericScalar::from_type_id(lhs_type, environment);
    let rhs_numeric = NumericScalar::from_type_id(rhs_type, environment);
    match (lhs_numeric, rhs_numeric) {
        (Some(lhs), Some(rhs)) => {
            if matches!(lhs, NumericScalar::Number(_)) || matches!(rhs, NumericScalar::Number(_)) {
                return Err(lir_transformation_error(
                    "Wasm lowering does not support Dec comparisons",
                ));
            }
            if matches!(
                (lhs, rhs),
                (NumericScalar::Int, NumericScalar::Float)
                    | (NumericScalar::Float, NumericScalar::Int)
            ) {
                return Err(lir_transformation_error(
                    "Wasm lowering received a mixed Int/Float comparison without its HIR conversion",
                ));
            }
            if !comparison_supported(lhs, rhs) {
                return Err(lir_transformation_error(format!(
                    "Wasm lowering received unsupported scalar comparison {lhs:?} {op:?} {rhs:?}"
                )));
            }
            let profile = context.module_context.request.numeric_profile;
            Ok(Some((
                comparison_type(lhs, profile),
                comparison_type(rhs, profile),
            )))
        }
        (None, None)
            if environment.fixed_scalar(lhs_type) == Some(FixedScalar::Byte)
                && environment.fixed_scalar(rhs_type) == Some(FixedScalar::Byte) =>
        {
            Ok(Some((
                WasmScalarComparisonType::UnsignedInteger(8),
                WasmScalarComparisonType::UnsignedInteger(8),
            )))
        }
        (None, None) => Ok(None),
        _ => Err(lir_transformation_error(format!(
            "Wasm lowering received incompatible scalar comparison operands for {op:?}"
        ))),
    }
}

fn comparison_type(
    scalar: NumericScalar,
    profile: moth_lexical::numeric::profile::NumericProfile,
) -> WasmScalarComparisonType {
    match scalar {
        NumericScalar::Int => WasmScalarComparisonType::SignedInteger(match profile.int_width {
            IntWidth::Bits32 => 32,
            IntWidth::Bits64 => 64,
        }),
        NumericScalar::Float => WasmScalarComparisonType::Float(match profile.float_precision {
            FloatPrecision::Bits32 => 32,
            FloatPrecision::Bits64 => 64,
        }),
        // Canonical F16 values use exact F32 carriers, which preserve their comparison semantics.
        NumericScalar::Fixed(FixedScalar::F16) => WasmScalarComparisonType::Float(32),
        NumericScalar::Fixed(scalar) => match scalar.class() {
            FixedScalarClass::SignedInteger => {
                WasmScalarComparisonType::SignedInteger(scalar.bit_width() as u8)
            }
            FixedScalarClass::UnsignedInteger => {
                WasmScalarComparisonType::UnsignedInteger(scalar.bit_width() as u8)
            }
            FixedScalarClass::BinaryFloat => {
                WasmScalarComparisonType::Float(scalar.bit_width() as u8)
            }
            FixedScalarClass::Octet => {
                unreachable!("Byte is not a NumericScalar")
            }
        },
        NumericScalar::Number(_) => {
            unreachable!("Number comparisons are rejected before ABI mapping")
        }
    }
}

fn scalar_comparison_op(op: HirBinOp) -> WasmScalarComparisonOp {
    match op {
        HirBinOp::Eq => WasmScalarComparisonOp::Eq,
        HirBinOp::Ne => WasmScalarComparisonOp::Ne,
        HirBinOp::Lt => WasmScalarComparisonOp::Lt,
        HirBinOp::Le => WasmScalarComparisonOp::Le,
        HirBinOp::Gt => WasmScalarComparisonOp::Gt,
        HirBinOp::Ge => WasmScalarComparisonOp::Ge,
        _ => unreachable!("only scalar comparisons use this mapper"),
    }
}

fn binop_unsupported_abi_error(op: &str, abi: WasmAbiType) -> CompilerError {
    lir_transformation_error(format!(
        "Wasm lowering does not support {op} for ABI type {abi:?}"
    ))
}

fn lower_string_concat_expression(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    expression: &HirExpression,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<ExprLoweringOutput, CompilerError> {
    // WHAT: lower compiler-owned StringAppend chains into explicit buffer operations.
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
                let abi = expression_abi(context, chunk);
                let chunk_handle = if chunk.ty
                    == context.module_context.type_environment.builtins().int
                {
                    if !matches!(abi, WasmAbiType::I32 | WasmAbiType::I64) {
                        return Err(lir_transformation_error(format!(
                            "Wasm lowering expected an Int carrier for string concatenation, found {abi:?}"
                        )));
                    }
                    let converted =
                        context.alloc_local(None, WasmAbiType::Handle, WasmLocalRole::ValueHandle);
                    statements.push(WasmLirStmt::StringFromI64 {
                        dst: converted,
                        value: lowered.value,
                    });
                    converted
                } else {
                    match abi {
                        WasmAbiType::Handle => lowered.value,
                        other => {
                            return Err(lir_transformation_error(format!(
                                "Wasm lowering string concatenation requires handle-compatible chunks, found {other:?}"
                            )));
                        }
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
        && matches!(op, HirBinOp::StringAppend)
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
