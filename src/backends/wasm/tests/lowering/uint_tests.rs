//! Uint scalar lowering for the Wasm backend.
//!
//! WHAT: pins HIR -> Wasm LIR lowering for profile-sized Uint literals, parameters, locals,
//!       results, unsigned and Uint/Int-mixed comparisons, infallible numeric casts and
//!       unsigned integer formatting.
//! WHY: Uint shares the Int width's I32/I64 carriers with an unsigned interpretation, so these
//!      tests prove the lowerer selects unsigned opcodes and preserves bit patterns above the
//!      signed maximum instead of reinterpreting values as signed.

use super::test_support::{
    build_module, build_type_environment, default_borrow_facts, default_numeric_proofs, expression,
    load_local, local, statement,
};
use crate::backends::wasm::backend::lower_hir_to_wasm_lir;
use crate::backends::wasm::lir::instructions::{
    WasmLirStmt, WasmScalarComparisonOp, WasmScalarComparisonType,
};
use crate::backends::wasm::lir::types::WasmAbiType;
use crate::backends::wasm::request::WasmBackendRequest;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::operators::HirBinOp;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};

fn uint_literal(expressions: &mut HirExpressionStore, value: u64) -> HirValueId {
    expression(
        HirExpressionKind::Uint(value),
        builtin_type_ids::UINT,
        RegionId(0),
        ValueKind::Const,
        expressions,
    )
}

fn int32_profile() -> NumericProfile {
    NumericProfile {
        int_width: IntWidth::Bits32,
        float_precision: FloatPrecision::Bits64,
    }
}

fn int64_profile() -> NumericProfile {
    NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits64,
    }
}

fn lower_entry_returning(
    expressions: HirExpressionStore,
    value: HirValueId,
    return_type: crate::compiler_frontend::datatypes::ids::TypeId,
    profile: NumericProfile,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
    type_environment: &crate::compiler_frontend::datatypes::environment::TypeEnvironment,
) -> crate::backends::wasm::lir::function::WasmLirFunction {
    let start_path = path_fork
        .try_intern_portable_path("main", string_table)
        .expect("test path fits");
    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(value),
    };
    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type,
    };
    let module = build_module(
        path_fork,
        string_table,
        expressions,
        vec![(start_function, start_path, HirFunctionOrigin::EntryStart)],
        vec![start_block],
        FunctionId(0),
    );
    let request = WasmBackendRequest {
        numeric_profile: profile,
        ..Default::default()
    };
    let lowered = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &request,
        string_table,
        type_environment,
        &path_fork.snapshot_table(),
    )
    .unwrap_or_else(|error| panic!("Uint expression should lower under {profile:?}: {error:?}"));
    lowered
        .lir_module
        .functions
        .into_iter()
        .next()
        .expect("the start function should be lowered")
}

#[test]
fn uint_literals_lower_with_bit_pattern_preservation() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, _) = build_type_environment();

    // Values above the signed maximum keep their exact bit patterns in the selected carrier.
    let cases = [
        (int32_profile(), 0u64, WasmAbiType::I32),
        (int32_profile(), 2_147_483_647, WasmAbiType::I32),
        (int32_profile(), 2_147_483_648, WasmAbiType::I32),
        (int32_profile(), 4_294_967_295, WasmAbiType::I32),
        (int64_profile(), 0, WasmAbiType::I64),
        (int64_profile(), 9_223_372_036_854_775_807, WasmAbiType::I64),
        (int64_profile(), 9_223_372_036_854_775_808, WasmAbiType::I64),
        (
            int64_profile(),
            18_446_744_073_709_551_615,
            WasmAbiType::I64,
        ),
    ];
    for (profile, value, carrier) in cases {
        let mut expressions = HirExpressionStore::default();
        let value_id = uint_literal(&mut expressions, value);
        let function = lower_entry_returning(
            expressions,
            value_id,
            builtin_type_ids::UINT,
            profile,
            &mut path_fork,
            &mut string_table,
            &type_environment,
        );
        let expected = match carrier {
            WasmAbiType::I32 => WasmLirStmt::ConstI32 {
                dst: function.blocks[0]
                    .statements
                    .iter()
                    .find_map(|statement| match statement {
                        WasmLirStmt::ConstI32 { dst, .. } => Some(*dst),
                        _ => None,
                    })
                    .expect("Uint32 literal should lower to ConstI32"),
                value: value as u32 as i32,
            },
            WasmAbiType::I64 => WasmLirStmt::ConstI64 {
                dst: function.blocks[0]
                    .statements
                    .iter()
                    .find_map(|statement| match statement {
                        WasmLirStmt::ConstI64 { dst, .. } => Some(*dst),
                        _ => None,
                    })
                    .expect("Uint64 literal should lower to ConstI64"),
                value: value as i64,
            },
            _ => unreachable!("Uint fixtures use integer carriers"),
        };
        assert!(
            function.blocks[0].statements.contains(&expected),
            "{profile:?} Uint literal {value} must preserve its bit pattern"
        );
    }
}

#[test]
fn uint_params_locals_and_results_use_the_selected_carrier() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, _) = build_type_environment();

    for profile in [int32_profile(), int64_profile()] {
        let mut expressions = HirExpressionStore::default();
        let expected = match profile.int_width {
            IntWidth::Bits32 => WasmAbiType::I32,
            IntWidth::Bits64 => WasmAbiType::I64,
        };
        let function_path = path_fork
            .try_intern_portable_path("identity", &mut string_table)
            .expect("test path fits");
        let block = HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![
                local(0, builtin_type_ids::UINT, RegionId(0)),
                local(1, builtin_type_ids::UINT, RegionId(0)),
            ],
            statements: vec![statement(
                1,
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(LocalId(1)),
                    value: load_local(
                        &mut expressions,
                        LocalId(0),
                        builtin_type_ids::UINT,
                        RegionId(0),
                    ),
                },
                1,
            )],
            terminator: HirTerminator::Return(load_local(
                &mut expressions,
                LocalId(1),
                builtin_type_ids::UINT,
                RegionId(0),
            )),
        };
        let function = HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![LocalId(0)],
            return_type: builtin_type_ids::UINT,
        };
        let module = build_module(
            &mut path_fork,
            &mut string_table,
            expressions,
            vec![(function, function_path, HirFunctionOrigin::Normal)],
            vec![block],
            FunctionId(0),
        );
        let request = WasmBackendRequest {
            numeric_profile: profile,
            ..Default::default()
        };
        let lowered = lower_hir_to_wasm_lir(
            &module,
            &default_borrow_facts(),
            &default_numeric_proofs(),
            &request,
            &string_table,
            &type_environment,
            &path_fork.snapshot_table(),
        )
        .unwrap_or_else(|error| panic!("Uint identity should lower under {profile:?}: {error:?}"));
        let lowered = lowered
            .lir_module
            .functions
            .into_iter()
            .next()
            .expect("the identity function should be lowered");
        assert_eq!(lowered.signature.params, vec![expected]);
        assert_eq!(lowered.signature.results, vec![expected]);
    }
}

#[test]
fn uint_comparisons_lower_with_unsigned_scalar_types() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();

    // (profile, operator, left_is_uint, expected left/right scalar types)
    let cases = [
        (HirBinOp::Eq, true, true),
        (HirBinOp::Ne, true, true),
        (HirBinOp::Lt, true, true),
        (HirBinOp::Le, true, true),
        (HirBinOp::Gt, true, true),
        (HirBinOp::Ge, true, true),
        (HirBinOp::Eq, false, true),
        (HirBinOp::Lt, false, true),
        (HirBinOp::Gt, true, false),
        (HirBinOp::Ge, false, true),
    ];
    for profile in [int32_profile(), int64_profile()] {
        let bits = match profile.int_width {
            IntWidth::Bits32 => 32,
            IntWidth::Bits64 => 64,
        };
        for (operator, left_is_uint, right_is_uint) in cases {
            let mut expressions = HirExpressionStore::default();
            let left = if left_is_uint {
                uint_literal(&mut expressions, 4_294_967_295)
            } else {
                expression(
                    HirExpressionKind::Int(-1),
                    types.int,
                    RegionId(0),
                    ValueKind::Const,
                    &mut expressions,
                )
            };
            let right = if right_is_uint {
                uint_literal(&mut expressions, 0)
            } else {
                expression(
                    HirExpressionKind::Int(-1),
                    types.int,
                    RegionId(0),
                    ValueKind::Const,
                    &mut expressions,
                )
            };
            let comparison = expression(
                HirExpressionKind::BinOp {
                    left,
                    op: operator,
                    right,
                },
                types.boolean,
                RegionId(0),
                ValueKind::RValue,
                &mut expressions,
            );
            let function = lower_entry_returning(
                expressions,
                comparison,
                types.boolean,
                profile,
                &mut path_fork,
                &mut string_table,
                &type_environment,
            );
            let expected_left = if left_is_uint {
                WasmScalarComparisonType::UnsignedInteger(bits)
            } else {
                WasmScalarComparisonType::SignedInteger(bits)
            };
            let expected_right = if right_is_uint {
                WasmScalarComparisonType::UnsignedInteger(bits)
            } else {
                WasmScalarComparisonType::SignedInteger(bits)
            };
            let expected_op = match operator {
                HirBinOp::Eq => WasmScalarComparisonOp::Eq,
                HirBinOp::Ne => WasmScalarComparisonOp::Ne,
                HirBinOp::Lt => WasmScalarComparisonOp::Lt,
                HirBinOp::Le => WasmScalarComparisonOp::Le,
                HirBinOp::Gt => WasmScalarComparisonOp::Gt,
                HirBinOp::Ge => WasmScalarComparisonOp::Ge,
                _ => unreachable!("Uint fixtures use comparison operators"),
            };
            assert!(
                function.blocks[0]
                    .statements
                    .iter()
                    .any(|statement| matches!(
                        statement,
                        WasmLirStmt::ScalarCompare {
                            op,
                            lhs_type,
                            rhs_type,
                            ..
                        } if *op == expected_op
                            && *lhs_type == expected_left
                            && *rhs_type == expected_right
                    )),
                "{profile:?} {operator:?} comparison must keep its signedness per operand"
            );
        }
    }
}

#[test]
fn uint_infallible_casts_use_unsigned_conversion_opcodes() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, _) = build_type_environment();
    let float32_profile = NumericProfile {
        int_width: IntWidth::Bits32,
        float_precision: FloatPrecision::Bits32,
    };

    // Uint converts directly to the profile Float without a signed intermediate.
    for profile in [int32_profile(), int64_profile(), float32_profile] {
        let mut expressions = HirExpressionStore::default();
        let source = uint_literal(&mut expressions, 3_000_000_001);
        let cast = expression(
            HirExpressionKind::Cast {
                source,
                policy: BuiltinCastPolicyId::NumericConversion {
                    source: NumericScalar::Uint,
                    target: NumericScalar::Float,
                },
            },
            builtin_type_ids::FLOAT,
            RegionId(0),
            ValueKind::RValue,
            &mut expressions,
        );
        let function = lower_entry_returning(
            expressions,
            cast,
            builtin_type_ids::FLOAT,
            profile,
            &mut path_fork,
            &mut string_table,
            &type_environment,
        );
        let expected_dst = match profile.float_precision {
            FloatPrecision::Bits32 => WasmAbiType::F32,
            FloatPrecision::Bits64 => WasmAbiType::F64,
        };
        assert!(
            function.blocks[0]
                .statements
                .iter()
                .any(|statement| matches!(
                    statement,
                    WasmLirStmt::IntegerToFloat {
                        source_signed: false,
                        ..
                    } if matches!(
                        statement,
                        WasmLirStmt::IntegerToFloat { dst, .. }
                        if function
                            .locals
                            .iter()
                            .find(|local| local.id == *dst)
                            .is_some_and(|local| local.ty == expected_dst)
                    )
                )),
            "{profile:?} Uint-to-Float must convert directly with an unsigned opcode"
        );
    }

    // A Uint32 value widens into U64 with zero extension, never sign extension.
    let mut expressions = HirExpressionStore::default();
    let source = uint_literal(&mut expressions, 4_294_967_295);
    let cast = expression(
        HirExpressionKind::Cast {
            source,
            policy: BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Uint,
                target: NumericScalar::Fixed(moth_lexical::numeric::fixed_scalar::FixedScalar::U64),
            },
        },
        builtin_type_ids::fixed_scalar(moth_lexical::numeric::fixed_scalar::FixedScalar::U64),
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );
    let function = lower_entry_returning(
        expressions,
        cast,
        builtin_type_ids::fixed_scalar(moth_lexical::numeric::fixed_scalar::FixedScalar::U64),
        int32_profile(),
        &mut path_fork,
        &mut string_table,
        &type_environment,
    );
    assert!(
        function.blocks[0]
            .statements
            .iter()
            .any(|statement| matches!(
                statement,
                WasmLirStmt::IntegerExtend {
                    source_signed: false,
                    ..
                }
            )),
        "Uint-to-U64 must zero-extend under the Int32 profile"
    );

    // Same-carrier Uint-to-U32 is a bit-pattern passthrough with no conversion opcode.
    let mut expressions = HirExpressionStore::default();
    let source = uint_literal(&mut expressions, 4_294_967_295);
    let cast = expression(
        HirExpressionKind::Cast {
            source,
            policy: BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Uint,
                target: NumericScalar::Fixed(moth_lexical::numeric::fixed_scalar::FixedScalar::U32),
            },
        },
        builtin_type_ids::fixed_scalar(moth_lexical::numeric::fixed_scalar::FixedScalar::U32),
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );
    let function = lower_entry_returning(
        expressions,
        cast,
        builtin_type_ids::fixed_scalar(moth_lexical::numeric::fixed_scalar::FixedScalar::U32),
        int32_profile(),
        &mut path_fork,
        &mut string_table,
        &type_environment,
    );
    assert!(
        function.blocks[0]
            .statements
            .iter()
            .any(|statement| matches!(statement, WasmLirStmt::ConstI32 { value: -1, .. })),
        "Uint32 maximum must keep its bit pattern through the U32 passthrough"
    );
    assert!(
        !function.blocks[0]
            .statements
            .iter()
            .any(|statement| matches!(
                statement,
                WasmLirStmt::IntegerToFloat { .. }
                    | WasmLirStmt::IntegerExtend { .. }
                    | WasmLirStmt::FloatExtend { .. }
                    | WasmLirStmt::RoundF16 { .. }
            )),
        "same-carrier Uint-to-U32 must not emit a conversion"
    );
}

#[test]
fn uint_to_string_formats_through_the_unsigned_helper() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();

    let cases = [
        (int32_profile(), 4_294_967_295, WasmAbiType::I32),
        (
            int64_profile(),
            18_446_744_073_709_551_615,
            WasmAbiType::I64,
        ),
    ];
    for (profile, value, carrier) in cases {
        let mut expressions = HirExpressionStore::default();
        let source = uint_literal(&mut expressions, value);
        let cast = expression(
            HirExpressionKind::Cast {
                source,
                policy: BuiltinCastPolicyId::NumericToString(NumericScalar::Uint),
            },
            types.string,
            RegionId(0),
            ValueKind::RValue,
            &mut expressions,
        );
        let function = lower_entry_returning(
            expressions,
            cast,
            types.string,
            profile,
            &mut path_fork,
            &mut string_table,
            &type_environment,
        );
        assert!(
            function.blocks[0]
                .statements
                .iter()
                .any(|statement| matches!(statement, WasmLirStmt::StringFromU64 { .. })),
            "{profile:?} Uint formatting must use the unsigned text helper"
        );
        assert!(
            !function.blocks[0]
                .statements
                .iter()
                .any(|statement| matches!(statement, WasmLirStmt::StringFromI64 { .. })),
            "{profile:?} Uint formatting must not use the signed text helper"
        );
        let _ = carrier;
    }
}

#[test]
fn uint_trap_arithmetic_selects_unsigned_operation_kinds() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, _) = build_type_environment();

    let operators = [
        NumericOperator::Add,
        NumericOperator::Subtract,
        NumericOperator::Multiply,
        NumericOperator::IntegerDivide,
        NumericOperator::Remainder,
        NumericOperator::Power,
    ];
    for profile in [int32_profile(), int64_profile()] {
        let expected_kind = match profile.int_width {
            IntWidth::Bits32 => {
                crate::backends::wasm::lir::instructions::WasmIntegerOperationKind::Unsigned32
            }
            IntWidth::Bits64 => {
                crate::backends::wasm::lir::instructions::WasmIntegerOperationKind::Unsigned64
            }
        };
        for operator in operators {
            let mut expressions = HirExpressionStore::default();
            let function_path = path_fork
                .try_intern_portable_path("operate", &mut string_table)
                .expect("test path fits");
            let operands = HirNumericOperands::Binary {
                left: uint_literal(&mut expressions, 7),
                right: uint_literal(&mut expressions, 3),
            };
            let block = HirBlock {
                id: BlockId(0),
                region: RegionId(0),
                locals: vec![local(0, builtin_type_ids::UINT, RegionId(0))],
                statements: vec![statement(
                    902,
                    HirStatementKind::NumericOp {
                        op: HirNumericOp {
                            operator,
                            domain: NumericScalar::Uint,
                        },
                        failure_mode: NumericFailureMode::Trap,
                        operands,
                        result: HirLocalDestination::Define(LocalId(0)),
                    },
                    902,
                )],
                terminator: HirTerminator::Return(load_local(
                    &mut expressions,
                    LocalId(0),
                    builtin_type_ids::UINT,
                    RegionId(0),
                )),
            };
            let function = HirFunction {
                id: FunctionId(0),
                entry: BlockId(0),
                params: vec![],
                return_type: builtin_type_ids::UINT,
            };
            let module = build_module(
                &mut path_fork,
                &mut string_table,
                expressions,
                vec![(function, function_path, HirFunctionOrigin::Normal)],
                vec![block],
                FunctionId(0),
            );
            let request = WasmBackendRequest {
                numeric_profile: profile,
                ..Default::default()
            };
            let lowered = lower_hir_to_wasm_lir(
                &module,
                &default_borrow_facts(),
                &default_numeric_proofs(),
                &request,
                &string_table,
                &type_environment,
                &path_fork.snapshot_table(),
            )
            .unwrap_or_else(|error| {
                panic!("{profile:?} Uint {operator:?} should lower: {error:?}")
            });
            let lowered = lowered
                .lir_module
                .functions
                .into_iter()
                .next()
                .expect("the operation function should be lowered");
            assert!(
                lowered.blocks[0]
                    .statements
                    .iter()
                    .any(|statement| matches!(
                        statement,
                        WasmLirStmt::CheckedIntegerOp { operation }
                            if operation.operator == operator && operation.kind == expected_kind
                    )),
                "{profile:?} Uint {operator:?} must lower as {expected_kind:?}"
            );
        }
    }
}
