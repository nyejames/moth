//! Statement lowering for HIR -> Wasm LIR.

use crate::backends::wasm::hir_to_lir::context::{WasmFunctionLoweringContext, lower_type_to_abi};
use crate::backends::wasm::hir_to_lir::expr::{expression_type, lower_expression};
use crate::backends::wasm::hir_to_lir::imports::resolve_host_call_import;
use crate::backends::wasm::lir::instructions::{
    IntegerCheckMode, WasmCalleeRef, WasmCheckedIntegerOperation, WasmIntegerOperationKind,
    WasmIntegerPowerScratch, WasmIntegerRangeCandidate, WasmIntegerRangeCandidateScratch,
    WasmIntegerScratch, WasmLirStmt, WasmNumericOperationOperands, WasmRangeCandidateKind,
};
use crate::backends::wasm::lir::types::{WasmAbiType, WasmLirLocalId};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::ids::{HirNodeId, HirValueId, LocalId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind, HirWriteTarget};
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::precision::BinaryFloatPrecision;

pub(crate) fn lower_statement(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    statement: &HirStatement,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), InfrastructureFailure> {
    // Statement lowering is explicitly side-effecting: expressions append LIR
    // statements directly to preserve HIR evaluation order.
    match &statement.kind {
        HirStatementKind::Write { target, value } => match target {
            HirWriteTarget::DefineLocal(local) => {
                lower_assignment(context, HirPlace::local(*local), *value, statements)
            }
            HirWriteTarget::AssignPlace(place) => {
                lower_assignment(context, *place, *value, statements)
            }
        },
        HirStatementKind::Call {
            target,
            args,
            results,
        } => {
            let args = context.module_context.hir_module.expressions.values(*args);
            let mut lowered_args = Vec::with_capacity(args.len());
            for arg in args {
                let lowered = lower_expression(context, *arg, statements)?;
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
                            InfrastructureFailure::lir_transformation(format!(
                                "Wasm lowering missing function id mapping for {function_id:?}"
                            ))
                        })?;
                    WasmCalleeRef::Function(function_id)
                }
                // Unreachable: `validate_wasm_cross_module_calls` rejects every reachable
                // cross-module call (`CrossModuleCalls`) before lowering.
                CallTarget::CrossModule(origin) => crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "Wasm lowering received unresolved cross-module function target {origin:?}"
                ),
                // Unreachable: `validate_hir_linked_function_targets` rejects module-private and
                // generated call targets (`LinkedFunctionTargets`) before lowering.
                CallTarget::ModulePrivate(identity) => crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "Wasm lowering received unresolved module-private function target {identity:?}"
                ),
                CallTarget::Generated(identity) => crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "Wasm lowering received unresolved generated function target {identity:?}"
                ),
                CallTarget::External(_) => {
                    // Host calls lower to deterministic import ids.
                    let import_id = resolve_host_call_import(context.module_context, target)?;
                    WasmCalleeRef::Import(import_id)
                }
            };

            let mut lowered_results = Vec::with_capacity(results.len());
            for destination in results {
                let local = context
                    .local_map
                    .get(&destination.local())
                    .copied()
                    .ok_or_else(|| {
                        InfrastructureFailure::lir_transformation(format!(
                            "Wasm lowering missing call result local {:?}",
                            destination.local()
                        ))
                    })?;
                lowered_results.push(local);
            }

            statements.push(WasmLirStmt::Call {
                results: lowered_results,
                callee,
                args: lowered_args,
            });

            Ok(())
        }
        HirStatementKind::MapOp { .. } => {
            // Unreachable: `validate_wasm_maps` rejects every reachable map use recorded in
            // `reachable_map_uses` (one per MapOp statement) before lowering.
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "Wasm hashmap operation reached lowering before backend feature validation"
            )
        }
        HirStatementKind::NumericOp {
            op,
            failure_mode,
            operands,
            result,
        } => {
            if matches!(op.domain, NumericScalar::Number(_)) {
                // Unreachable: `validate_wasm_number_values` rejects every reachable Dec-typed
                // NumericOp operand or result before lowering.
                crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "Wasm lowering does not support Dec arithmetic"
                );
            }

            if op.domain.is_integer() {
                lower_checked_integer_operation(
                    context,
                    statement.id,
                    *op,
                    *failure_mode,
                    operands,
                    result.local(),
                    statements,
                )
            } else {
                lower_checked_float_operation(
                    context,
                    *op,
                    *failure_mode,
                    operands,
                    result.local(),
                    statements,
                )
            }
        }
        // Trap-mode loop-step statements reach here: the Wasm feature validator
        // rejects only recoverable `Invoke` guards, so this stays infrastructure.
        HirStatementKind::LoopStepFailure { .. } => Err(InfrastructureFailure::lir_transformation(
            "Loop-step failure must be lowered as a block terminator",
        )),
        HirStatementKind::RangeCandidate {
            current,
            step,
            end,
            inclusive,
            domain,
            candidate_result,
            in_range_result,
        } => lower_range_candidate(
            context,
            [*current, *step, *end],
            *inclusive,
            *domain,
            [candidate_result.local(), in_range_result.local()],
            statements,
        ),
        HirStatementKind::FormatFloat {
            source,
            failure_mode,
            result,
        } => lower_format_float(context, *source, *failure_mode, result.local(), statements),
        HirStatementKind::ValidateFloat {
            source,
            failure_mode,
            result,
        } => lower_validate_float(context, *source, *failure_mode, result.local(), statements),
        HirStatementKind::Expr(expression) => {
            let _ = lower_expression(context, *expression, statements)?;
            Ok(())
        }
        HirStatementKind::PushRuntimeFragment { vec_local, value } => {
            // WHAT: lower a runtime fragment push into a Wasm vec-push sequence.
            // WHY: entry start() accumulates runtime fragments via PushRuntimeFragment;
            //      the Wasm backend must append the evaluated string to the fragment vec.
            lower_push_runtime_fragment(context, vec_local, *value, statements)
        }
    }
}

fn lower_checked_integer_operation(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    statement: HirNodeId,
    op: HirNumericOp,
    failure_mode: NumericFailureMode,
    operands: &HirNumericOperands,
    result: LocalId,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), InfrastructureFailure> {
    if !matches!(
        failure_mode,
        NumericFailureMode::Trap | NumericFailureMode::Infallible
    ) || !op.domain.is_integer()
    {
        crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm lowering does not support checked numeric operation {op} in {failure_mode:?} mode"
        );
    }

    let kind = integer_operation_kind(op.domain)?;
    // Proven-safe operations keep evaluation order, domain and signedness while losing only
    // the redundant runtime predicates. Every other mode retains the checked lowering below.
    let check_mode = if context
        .module_context
        .numeric_proofs
        .integer_operation_is_safe(statement)
    {
        IntegerCheckMode::ProvenSafe
    } else {
        IntegerCheckMode::Runtime
    };
    let lir_operands = match (op.operator, operands) {
        (NumericOperator::Negate, HirNumericOperands::Unary { operand }) => {
            let lowered = lower_expression(context, *operand, statements)?;
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
            let lowered_left = lower_expression(context, *left, statements)?;
            let lowered_right = lower_expression(context, *right, statements)?;
            WasmNumericOperationOperands::Binary {
                left: lowered_left.value,
                right: lowered_right.value,
            }
        }
        _ => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm lowering received invalid integer numeric operands for {op}"
        ),
    };

    let Some(destination) = context.local_map.get(&result).copied() else {
        return Err(InfrastructureFailure::lir_transformation(format!(
            "Wasm lowering could not resolve numeric result local {result:?}"
        )));
    };
    let expected_carrier = kind.carrier();
    let destination_carrier = context.local_type_by_id.get(&destination).copied();
    if destination_carrier != Some(expected_carrier) {
        return Err(InfrastructureFailure::lir_transformation(format!(
            "Wasm lowering expected numeric result carrier {expected_carrier:?}, found {destination_carrier:?}"
        )));
    }

    // Checked lowering keeps an operand's old value available to overflow predicates and commits
    // the assignment only after the checked operation succeeds. Proven-safe native arithmetic
    // evaluates every operand before its `local.set`, so it can write an aliasing destination
    // directly without a temp or copy.
    let destination_aliases_source = match lir_operands {
        WasmNumericOperationOperands::Unary { operand } => destination == operand,
        WasmNumericOperationOperands::Binary { left, right } => {
            destination == left || destination == right
        }
    };
    let operation_destination =
        if destination_aliases_source && check_mode == IntegerCheckMode::Runtime {
            context.alloc_temp(expected_carrier)
        } else {
            destination
        };

    let product_scratch = match op.operator {
        NumericOperator::Power => {
            // Power reuses the widened product across repeated factor accumulations.
            if uses_32_bit_product_scratch(kind) {
                Some(context.alloc_temp(WasmAbiType::I64))
            } else {
                None
            }
        }
        // A runtime 32-bit multiply needs the widened product for its range predicates; a
        // proven-safe multiply stays inside its semantic domain, so a check-only widened
        // product scratch is omitted.
        NumericOperator::Multiply
            if check_mode == IntegerCheckMode::Runtime && uses_32_bit_product_scratch(kind) =>
        {
            Some(context.alloc_temp(WasmAbiType::I64))
        }
        _ => None,
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
        operation: WasmCheckedIntegerOperation {
            dst: operation_destination,
            operator: op.operator,
            kind,
            check_mode,
            operands: lir_operands,
            scratch,
        },
    });

    if destination_aliases_source && operation_destination != destination {
        statements.push(WasmLirStmt::Copy {
            dst: destination,
            src: operation_destination,
        });
    }

    Ok(())
}

fn lower_validate_float(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    source: HirValueId,
    failure_mode: NumericFailureMode,
    result: LocalId,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), InfrastructureFailure> {
    if failure_mode != NumericFailureMode::Trap {
        crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm lowering does not support binary-float validation in {failure_mode:?} mode"
        );
    }

    let type_environment = context.module_context.type_environment;
    let source_type = expression_type(context, source);
    // WHAT: the validated source owns the precision, so the guard reads the source's exact
    //       binary-float type instead of a compilation-boundary width.
    // WHY: a fixed F32/F64 boundary value must not be validated at another type's precision,
    //      and F16 stays an explicit unsupported route instead of being lowered as F32.
    let source_scalar = NumericScalar::from_type_id(source_type, type_environment)
        .filter(|scalar| scalar.is_binary_float())
        .unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "Wasm binary-float validation source has type {source_type:?}, which is not a binary float"
            )
        });
    let precision = source_scalar
        .binary_float_precision()
        .filter(|precision| {
            matches!(
                precision,
                BinaryFloatPrecision::Binary32 | BinaryFloatPrecision::Binary64
            )
        })
        .ok_or_else(|| {
            InfrastructureFailure::lir_transformation(format!(
                "Wasm binary-float validation requires an F32 or F64 source, found {source_scalar}"
            ))
        })?;
    let expected_carrier = match precision {
        BinaryFloatPrecision::Binary32 => WasmAbiType::F32,
        BinaryFloatPrecision::Binary64 => WasmAbiType::F64,
        BinaryFloatPrecision::Binary16 => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "binary-float validation already filtered Binary16 out of precision before this carrier match"
        ),
    };

    let destination = context.local_map.get(&result).copied().ok_or_else(|| {
        InfrastructureFailure::lir_transformation(format!(
            "Wasm lowering could not resolve binary-float validation result local {result:?}"
        ))
    })?;
    if context.local_type_by_id.get(&destination).copied() != Some(expected_carrier) {
        return Err(InfrastructureFailure::lir_transformation(format!(
            "Wasm binary-float validation destination does not use the {expected_carrier:?} carrier of the source precision"
        )));
    }

    // Lower the boundary value once; ValidateFloat's emitter reads it before writing the result,
    // so an in-place result can reuse the source local without a copy.
    let lowered_source = lower_expression(context, source, statements)?;
    if context.local_type_by_id.get(&lowered_source.value).copied() != Some(expected_carrier) {
        return Err(InfrastructureFailure::lir_transformation(format!(
            "Wasm binary-float validation source does not use the {expected_carrier:?} carrier of its own precision"
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
    source: HirValueId,
    failure_mode: NumericFailureMode,
    result: LocalId,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), InfrastructureFailure> {
    if failure_mode != NumericFailureMode::Trap {
        crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm lowering does not support binary-float formatting in {failure_mode:?} mode"
        );
    }

    let type_environment = context.module_context.type_environment;
    let source_type = expression_type(context, source);
    // WHAT: the formatted source owns its exact binary-float precision, so F16 formats through
    //       its F32 carrier at binary16 precision.
    // WHY: each binary-float type owns its exact precision.
    let source_scalar = NumericScalar::from_type_id(source_type, type_environment)
        .filter(|scalar| scalar.is_binary_float())
        .unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "Wasm binary-float formatting source has type {source_type:?}, which is not a binary float"
            )
        });
    let precision = source_scalar
        .binary_float_precision()
        .filter(|precision| {
            matches!(
                precision,
                BinaryFloatPrecision::Binary16
                    | BinaryFloatPrecision::Binary32
                    | BinaryFloatPrecision::Binary64
            )
        })
        .ok_or_else(|| {
            InfrastructureFailure::lir_transformation(format!(
                "Wasm binary-float formatting requires a binary float source, found {source_scalar}"
            ))
        })?;
    let expected_carrier = match precision {
        BinaryFloatPrecision::Binary16 | BinaryFloatPrecision::Binary32 => WasmAbiType::F32,
        BinaryFloatPrecision::Binary64 => WasmAbiType::F64,
    };

    let destination = context.local_map.get(&result).copied().ok_or_else(|| {
        InfrastructureFailure::lir_transformation(format!(
            "Wasm lowering could not resolve binary-float formatting result local {result:?}"
        ))
    })?;
    if context.local_type_by_id.get(&destination).copied() != Some(WasmAbiType::Handle) {
        return Err(InfrastructureFailure::lir_transformation(
            "Wasm binary-float formatting result does not use the String handle carrier",
        ));
    }

    // Evaluate the source once; its exact float precision remains explicit in LIR.
    let lowered_source = lower_expression(context, source, statements)?;
    if context.local_type_by_id.get(&lowered_source.value).copied() != Some(expected_carrier) {
        return Err(InfrastructureFailure::lir_transformation(format!(
            "Wasm binary-float formatting source does not use the carrier {expected_carrier:?} of its own precision"
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
) -> Result<(), InfrastructureFailure> {
    if failure_mode != NumericFailureMode::Trap || !op.domain.is_binary_float() {
        crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm lowering does not support checked float operation {op} in {failure_mode:?} mode"
        );
    }

    let precision = op
        .domain
        .binary_float_precision()
        .filter(|precision| {
            matches!(
                precision,
                BinaryFloatPrecision::Binary32 | BinaryFloatPrecision::Binary64
            )
        })
        .unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "Wasm checked float lowering does not support {} precision",
                op.domain
            )
        });
    let expected_type = op.domain.type_id(context.module_context.type_environment);

    let lir_operands = match (op.operator, operands) {
        (NumericOperator::Negate, HirNumericOperands::Unary { operand }) => {
            let operand_type = expression_type(context, *operand);
            if operand_type != expected_type {
                crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "Wasm checked float operand has type {:?}, expected {}",
                    operand_type,
                    op.domain
                );
            }
            let lowered = lower_expression(context, *operand, statements)?;
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
            if expression_type(context, *left) != expected_type
                || expression_type(context, *right) != expected_type
            {
                crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "Wasm checked float operands do not match {}",
                    op.domain
                );
            }

            // Linearise both operands in source order before emitting the checked operation.
            let lowered_left = lower_expression(context, *left, statements)?;
            let lowered_right = lower_expression(context, *right, statements)?;
            WasmNumericOperationOperands::Binary {
                left: lowered_left.value,
                right: lowered_right.value,
            }
        }
        _ => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm lowering received invalid float numeric operands for {op}"
        ),
    };

    let Some(destination) = context.local_map.get(&result).copied() else {
        return Err(InfrastructureFailure::lir_transformation(format!(
            "Wasm lowering could not resolve float numeric result local {result:?}"
        )));
    };
    let expected_carrier = match precision {
        BinaryFloatPrecision::Binary32 => WasmAbiType::F32,
        BinaryFloatPrecision::Binary64 => WasmAbiType::F64,
        BinaryFloatPrecision::Binary16 => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "checked float lowering already rejected Binary16 before this carrier match"
        ),
    };
    if context.local_type_by_id.get(&destination).copied() != Some(expected_carrier) {
        return Err(InfrastructureFailure::lir_transformation(format!(
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

fn lower_range_candidate(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    inputs: [HirValueId; 3],
    inclusive: bool,
    domain: NumericScalar,
    output_ids: [LocalId; 2],
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), InfrastructureFailure> {
    if domain.is_integer() {
        lower_integer_range_candidate(context, inputs, inclusive, domain, output_ids, statements)
    } else {
        lower_float_range_candidate(context, inputs, inclusive, domain, output_ids, statements)
    }
}

/// Lower one integer range candidate into the widened I64 distance algorithm.
///
/// WHAT: resolves the counter and independent signed-step carriers, allocates the widened I64
///       scratch, and emits one `RangeCandidate` statement.
/// WHY: the counter and the step keep separate carriers, and narrow counters ride an I32
///       carrier, so the emitter needs both carriers and the counter's signedness rather than a
///       single counter type.
fn lower_integer_range_candidate(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    inputs: [HirValueId; 3],
    inclusive: bool,
    domain: NumericScalar,
    output_ids: [LocalId; 2],
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), InfrastructureFailure> {
    let [current, step, end] = inputs;

    let Some((minimum, _)) = domain.integer_range() else {
        crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm integer range candidate requires a fixed integer counter domain, found {domain}"
        );
    };
    let counter_signed = minimum < 0;
    let counter_type = domain.type_id(context.module_context.type_environment);
    if expression_type(context, current) != counter_type
        || expression_type(context, end) != counter_type
    {
        crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm integer range candidate operands do not match {domain}"
        );
    }

    // The step keeps its own completed type and is validated as a signed fixed integer upstream.
    let step_type = expression_type(context, step);
    let Some(step_domain) =
        NumericScalar::from_type_id(step_type, context.module_context.type_environment)
    else {
        crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm integer range candidate step is not a fixed numeric type"
        );
    };
    if !matches!(step_domain.integer_range(), Some((minimum, _)) if minimum < 0) {
        crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm integer range candidate step is not a signed fixed integer: {step_domain}"
        );
    }

    let counter_carrier = lower_type_to_abi(context.module_context, counter_type);
    let step_carrier = lower_type_to_abi(context.module_context, step_type);

    let lowered_current = lower_expression(context, current, statements)?;
    let lowered_step = lower_expression(context, step, statements)?;
    let lowered_end = lower_expression(context, end, statements)?;
    for (operand, expected, label) in [
        (lowered_current.value, counter_carrier, "current"),
        (lowered_step.value, step_carrier, "step"),
        (lowered_end.value, counter_carrier, "end"),
    ] {
        if context.local_type_by_id.get(&operand).copied() != Some(expected) {
            return Err(InfrastructureFailure::lir_transformation(format!(
                "Wasm integer range candidate {label} does not use carrier {expected:?}"
            )));
        }
    }

    let [candidate_dst, in_range_dst] =
        resolve_range_candidate_destinations(context, output_ids, counter_carrier, domain)?;

    let scratch = WasmIntegerRangeCandidateScratch {
        current: context.alloc_temp(WasmAbiType::I64),
        step: context.alloc_temp(WasmAbiType::I64),
        end: context.alloc_temp(WasmAbiType::I64),
        ascending: context.alloc_temp(WasmAbiType::I32),
        distance: context.alloc_temp(WasmAbiType::I64),
        magnitude: context.alloc_temp(WasmAbiType::I64),
        candidate: context.alloc_temp(WasmAbiType::I64),
    };

    statements.push(WasmLirStmt::RangeCandidate {
        candidate_dst,
        in_range_dst,
        current: lowered_current.value,
        step: lowered_step.value,
        end: lowered_end.value,
        inclusive,
        kind: WasmRangeCandidateKind::Integer(WasmIntegerRangeCandidate {
            counter_carrier,
            counter_signed,
            step_carrier,
            scratch,
        }),
    });
    Ok(())
}

/// Lower one binary-float range candidate at the counter's exact precision.
fn lower_float_range_candidate(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    inputs: [HirValueId; 3],
    inclusive: bool,
    domain: NumericScalar,
    output_ids: [LocalId; 2],
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), InfrastructureFailure> {
    let [current, step, end] = inputs;
    // Binary16 is unreachable here: validate_wasm_range_candidate_domains rejects a reachable
    // F16 counter before lowering. Only Binary32 and Binary64 have native carriers.
    let precision = match domain.binary_float_precision() {
        Some(BinaryFloatPrecision::Binary16) => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm float range candidates do not support {} precision",
            domain
        ),
        Some(precision @ (BinaryFloatPrecision::Binary32 | BinaryFloatPrecision::Binary64)) => {
            precision
        }
        None => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm float range candidates do not support {} precision",
            domain
        ),
    };
    let expected_type = domain.type_id(context.module_context.type_environment);
    if expression_type(context, current) != expected_type
        || expression_type(context, step) != expected_type
        || expression_type(context, end) != expected_type
    {
        crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm float range candidate operands do not match {}",
            domain
        );
    }

    let expected_carrier = match precision {
        BinaryFloatPrecision::Binary32 => WasmAbiType::F32,
        BinaryFloatPrecision::Binary64 => WasmAbiType::F64,
        BinaryFloatPrecision::Binary16 => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "float range candidate precision match already rejected Binary16 before this carrier match"
        ),
    };
    let lowered_current = lower_expression(context, current, statements)?;
    let lowered_step = lower_expression(context, step, statements)?;
    let lowered_end = lower_expression(context, end, statements)?;
    for (operand, label) in [
        (lowered_current.value, "current"),
        (lowered_step.value, "step"),
        (lowered_end.value, "end"),
    ] {
        if context.local_type_by_id.get(&operand).copied() != Some(expected_carrier) {
            return Err(InfrastructureFailure::lir_transformation(format!(
                "Wasm float range candidate {label} does not use carrier {expected_carrier:?}"
            )));
        }
    }

    let [candidate_dst, in_range_dst] =
        resolve_range_candidate_destinations(context, output_ids, expected_carrier, domain)?;

    let scratch = context.alloc_temp(expected_carrier);
    statements.push(WasmLirStmt::RangeCandidate {
        candidate_dst,
        in_range_dst,
        current: lowered_current.value,
        step: lowered_step.value,
        end: lowered_end.value,
        inclusive,
        kind: WasmRangeCandidateKind::BinaryFloat { precision, scratch },
    });
    Ok(())
}

/// Resolve both range-candidate result locals and check their carriers.
fn resolve_range_candidate_destinations(
    context: &WasmFunctionLoweringContext<'_, '_>,
    output_ids: [LocalId; 2],
    candidate_carrier: WasmAbiType,
    domain: NumericScalar,
) -> Result<[WasmLirLocalId; 2], InfrastructureFailure> {
    let [candidate_result, in_range_result] = output_ids;
    let Some(candidate_dst) = context.local_map.get(&candidate_result).copied() else {
        return Err(InfrastructureFailure::lir_transformation(format!(
            "Wasm lowering could not resolve range candidate local {candidate_result:?} for {domain}"
        )));
    };
    let Some(in_range_dst) = context.local_map.get(&in_range_result).copied() else {
        return Err(InfrastructureFailure::lir_transformation(format!(
            "Wasm lowering could not resolve range candidate Bool local {in_range_result:?} for {domain}"
        )));
    };
    if context.local_type_by_id.get(&candidate_dst).copied() != Some(candidate_carrier) {
        return Err(InfrastructureFailure::lir_transformation(format!(
            "Wasm range candidate destination does not use carrier {candidate_carrier:?} for {domain}"
        )));
    }
    if context.local_type_by_id.get(&in_range_dst).copied() != Some(WasmAbiType::I32) {
        return Err(InfrastructureFailure::lir_transformation(
            "Wasm range candidate Bool result does not use the I32 carrier",
        ));
    }
    Ok([candidate_dst, in_range_dst])
}

/// Map one exact checked-integer domain onto the LIR kind Wasm implements.
///
/// The checked lowering emits 32/64-bit range predicates, so the supported domain set is exactly
/// `I32`/`U32`/`I64`/`U64`; narrower fixed integers have no checked path and stay behind the
/// backend capability gate.
fn integer_operation_kind(
    domain: NumericScalar,
) -> Result<WasmIntegerOperationKind, InfrastructureFailure> {
    match domain {
        NumericScalar::Fixed(FixedScalar::I32) => Ok(WasmIntegerOperationKind::Signed32),
        NumericScalar::Fixed(FixedScalar::U32) => Ok(WasmIntegerOperationKind::Unsigned32),
        NumericScalar::Fixed(FixedScalar::I64) => Ok(WasmIntegerOperationKind::Signed64),
        NumericScalar::Fixed(FixedScalar::U64) => Ok(WasmIntegerOperationKind::Unsigned64),
        _ => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm checked integer lowering does not implement the exact domain {domain}"
        ),
    }
}

/// Whether the widened I64 product scratch exists only for check-only range predicates.
fn uses_32_bit_product_scratch(kind: WasmIntegerOperationKind) -> bool {
    matches!(
        kind,
        WasmIntegerOperationKind::Signed32 | WasmIntegerOperationKind::Unsigned32
    )
}

fn lower_assignment(
    context: &mut WasmFunctionLoweringContext<'_, '_>,
    target: HirPlace,
    value: HirValueId,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), InfrastructureFailure> {
    // WHAT: preserve explicit move/copy distinction in LIR.
    // WHY: LIR keeps the move/copy choice explicit for ownership hooks; lowering does not
    // consume `ValidatedMemoryPlan`.
    if !context
        .module_context
        .hir_module
        .expressions
        .projections(target.projections)
        .is_empty()
    {
        // Unreachable: `validate_wasm_unsupported_shapes` rejects projected assignment targets
        // (`ProjectedAssignmentTargets`) before lowering.
        crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "Wasm lowering currently supports assignments only to direct locals"
        );
    }

    let dst = context
        .local_map
        .get(&target.root)
        .copied()
        .ok_or_else(|| {
            InfrastructureFailure::lir_transformation(format!(
                "Wasm lowering could not resolve assignment target local {:?}",
                target.root
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
    value: HirValueId,
    statements: &mut Vec<WasmLirStmt>,
) -> Result<(), InfrastructureFailure> {
    let vec_handle = context.local_map.get(vec_local).copied().ok_or_else(|| {
        InfrastructureFailure::lir_transformation(format!(
            "Wasm lowering could not resolve runtime fragment vec local {vec_local:?}",
        ))
    })?;

    if !context.is_handle_local(vec_handle) {
        return Err(InfrastructureFailure::lir_transformation(format!(
            "Wasm lowering expected runtime fragment vec local {vec_local:?} to lower as a handle",
        )));
    }

    let lowered_value = lower_expression(context, value, statements)?;
    if !context.is_handle_local(lowered_value.value) {
        return Err(InfrastructureFailure::lir_transformation(
            "Wasm lowering expected runtime fragment values to lower as string handles",
        ));
    }

    statements.push(WasmLirStmt::VecPushHandle {
        vec: vec_handle,
        handle: lowered_value.value,
    });

    Ok(())
}
