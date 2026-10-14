//! Unsigned integer scalar lowering for the Wasm backend.
//!
//! WHAT: pins HIR -> Wasm LIR lowering for exact-width U32/U64 literals, parameters,
//!       locals, results, unsigned and mixed-signedness comparisons, infallible numeric casts and
//!       unsigned integer formatting.
//! WHY: U32/U64 share the I32/I64 carriers with their signed counterparts under an
//!      unsigned interpretation, so these tests prove the lowerer selects unsigned opcodes and
//!      preserves bit patterns above the signed maximum instead of reinterpreting values as
//!      signed.

use super::test_support::{
    build_module, build_type_environment, default_borrow_facts, default_numeric_proofs, expression,
    fixed_scalar_type, load_local, local, statement,
};
use crate::backends::wasm::backend::lower_hir_to_wasm_lir;
use crate::backends::wasm::lir::instructions::{
    WasmIntegerOperationKind, WasmLirStmt, WasmScalarComparisonOp, WasmScalarComparisonType,
};
use crate::backends::wasm::lir::types::WasmAbiType;
use crate::backends::wasm::request::WasmBackendRequest;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
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
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

fn uint_literal(
    expressions: &mut HirExpressionStore,
    scalar: FixedScalar,
    value: u64,
) -> HirValueId {
    expression(
        HirExpressionKind::FixedScalar(
            FixedScalarValue::unsigned(scalar, value).expect("valid test value"),
        ),
        fixed_scalar_type(scalar),
        RegionId(0),
        ValueKind::Const,
        expressions,
    )
}

fn lower_entry_returning(
    mut expressions: HirExpressionStore,
    value: HirValueId,
    return_type: crate::compiler_frontend::datatypes::ids::TypeId,
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
        terminator: HirTerminator::Return(
            expressions
                .append_values(&[value], None)
                .expect("fixture return values should fit"),
        ),
    };
    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        returns: vec![return_type].into_boxed_slice(),
        error_return: None,
    };
    let module = build_module(
        path_fork,
        string_table,
        expressions,
        vec![(start_function, start_path, HirFunctionOrigin::EntryStart)],
        vec![start_block],
        FunctionId(0),
    );
    let request = WasmBackendRequest::default();
    let lowered = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &request,
        string_table,
        type_environment,
        &path_fork.snapshot_table(),
    )
    .unwrap_or_else(|error| panic!("unsigned integer expression should lower: {error:?}"));
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

    // Values above the signed maximum keep their exact bit patterns in the scalar's own carrier.
    let cases = [
        (FixedScalar::U32, 0u64, WasmAbiType::I32),
        (FixedScalar::U32, 2_147_483_647, WasmAbiType::I32),
        (FixedScalar::U32, 2_147_483_648, WasmAbiType::I32),
        (FixedScalar::U32, 4_294_967_295, WasmAbiType::I32),
        (FixedScalar::U64, 0, WasmAbiType::I64),
        (
            FixedScalar::U64,
            9_223_372_036_854_775_807,
            WasmAbiType::I64,
        ),
        (
            FixedScalar::U64,
            9_223_372_036_854_775_808,
            WasmAbiType::I64,
        ),
        (
            FixedScalar::U64,
            18_446_744_073_709_551_615,
            WasmAbiType::I64,
        ),
    ];
    for (scalar, value, carrier) in cases {
        let mut expressions = HirExpressionStore::default();
        let value_id = uint_literal(&mut expressions, scalar, value);
        let function = lower_entry_returning(
            expressions,
            value_id,
            fixed_scalar_type(scalar),
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
                    .expect("U32 literal should lower to ConstI32"),
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
                    .expect("U64 literal should lower to ConstI64"),
                value: value as i64,
            },
            _ => unreachable!("unsigned integer fixtures use integer carriers"),
        };
        assert!(
            function.blocks[0].statements.contains(&expected),
            "{scalar:?} literal {value} must preserve its bit pattern"
        );
    }
}

#[test]
fn uint_params_locals_and_results_use_the_exact_carrier() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, _) = build_type_environment();

    for (scalar, expected) in [
        (FixedScalar::U32, WasmAbiType::I32),
        (FixedScalar::U64, WasmAbiType::I64),
    ] {
        let mut expressions = HirExpressionStore::default();
        let scalar_type = fixed_scalar_type(scalar);
        let function_path = path_fork
            .try_intern_portable_path("identity", &mut string_table)
            .expect("test path fits");
        let block = HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![
                local(0, scalar_type, RegionId(0)),
                local(1, scalar_type, RegionId(0)),
            ],
            statements: vec![statement(
                1,
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(LocalId(1)),
                    value: load_local(&mut expressions, LocalId(0), scalar_type, RegionId(0)),
                },
                1,
            )],
            terminator: HirTerminator::Return({
                let value = load_local(&mut expressions, LocalId(1), scalar_type, RegionId(0));
                expressions
                    .append_values(&[value], None)
                    .expect("fixture return values should fit")
            }),
        };
        let function = HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![LocalId(0)],
            returns: vec![scalar_type].into_boxed_slice(),
            error_return: None,
        };
        let module = build_module(
            &mut path_fork,
            &mut string_table,
            expressions,
            vec![(function, function_path, HirFunctionOrigin::Normal)],
            vec![block],
            FunctionId(0),
        );
        let request = WasmBackendRequest::default();
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
            panic!("unsigned integer identity should lower with {scalar:?}: {error:?}")
        });
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

    // (operator, left_is_uint, right_is_uint)
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
    for (uint_scalar, signed_scalar, bits) in [
        (FixedScalar::U32, FixedScalar::I32, 32u8),
        (FixedScalar::U64, FixedScalar::I64, 64u8),
    ] {
        for (operator, left_is_uint, right_is_uint) in cases {
            let mut expressions = HirExpressionStore::default();
            let left = if left_is_uint {
                uint_literal(&mut expressions, uint_scalar, 4_294_967_295)
            } else {
                expression(
                    HirExpressionKind::FixedScalar(
                        FixedScalarValue::signed(signed_scalar, -1).expect("valid test value"),
                    ),
                    fixed_scalar_type(signed_scalar),
                    RegionId(0),
                    ValueKind::Const,
                    &mut expressions,
                )
            };
            let right = if right_is_uint {
                uint_literal(&mut expressions, uint_scalar, 0)
            } else {
                expression(
                    HirExpressionKind::FixedScalar(
                        FixedScalarValue::signed(signed_scalar, -1).expect("valid test value"),
                    ),
                    fixed_scalar_type(signed_scalar),
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
                _ => unreachable!("unsigned integer fixtures use comparison operators"),
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
                "{uint_scalar:?} {operator:?} comparison must keep its signedness per operand"
            );
        }
    }
}

#[test]
fn uint_infallible_casts_use_unsigned_conversion_opcodes() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, _) = build_type_environment();

    // U32/U64 convert directly to their exact binary-float target without a signed intermediate.
    for (source_scalar, target_scalar) in [
        (FixedScalar::U32, FixedScalar::F64),
        (FixedScalar::U64, FixedScalar::F64),
        (FixedScalar::U32, FixedScalar::F32),
    ] {
        let mut expressions = HirExpressionStore::default();
        let source = uint_literal(&mut expressions, source_scalar, 3_000_000_001);
        let target_type = fixed_scalar_type(target_scalar);
        let cast = expression(
            HirExpressionKind::Cast {
                source,
                policy: BuiltinCastPolicyId::NumericConversion {
                    source: NumericScalar::Fixed(source_scalar),
                    target: NumericScalar::Fixed(target_scalar),
                },
            },
            target_type,
            RegionId(0),
            ValueKind::RValue,
            &mut expressions,
        );
        let function = lower_entry_returning(
            expressions,
            cast,
            target_type,
            &mut path_fork,
            &mut string_table,
            &type_environment,
        );
        let expected_dst = match target_scalar {
            FixedScalar::F32 => WasmAbiType::F32,
            FixedScalar::F64 => WasmAbiType::F64,
            _ => unreachable!("numeric conversion targets are binary floats"),
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
            "{source_scalar:?}->{target_scalar:?} must convert with an unsigned opcode"
        );
    }

    // A U32 value widens into U64 with zero extension, never sign extension.
    let mut expressions = HirExpressionStore::default();
    let u64_type = fixed_scalar_type(FixedScalar::U64);
    let source = uint_literal(&mut expressions, FixedScalar::U32, 4_294_967_295);
    let cast = expression(
        HirExpressionKind::Cast {
            source,
            policy: BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Fixed(FixedScalar::U32),
                target: NumericScalar::Fixed(FixedScalar::U64),
            },
        },
        u64_type,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );
    let function = lower_entry_returning(
        expressions,
        cast,
        u64_type,
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
        "U32 must zero-extend into U64, never sign-extend"
    );

    // Same-carrier U16-to-U32 widening is a bit-pattern passthrough with no conversion
    // opcode: both scalars share the I32 carrier, so the value needs no instruction.
    let mut expressions = HirExpressionStore::default();
    let u32_type = fixed_scalar_type(FixedScalar::U32);
    let source = uint_literal(&mut expressions, FixedScalar::U16, u64::from(u16::MAX));
    let cast = expression(
        HirExpressionKind::Cast {
            source,
            policy: BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Fixed(FixedScalar::U16),
                target: NumericScalar::Fixed(FixedScalar::U32),
            },
        },
        u32_type,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );
    let function = lower_entry_returning(
        expressions,
        cast,
        u32_type,
        &mut path_fork,
        &mut string_table,
        &type_environment,
    );
    assert!(
        function.blocks[0]
            .statements
            .iter()
            .any(|statement| matches!(statement, WasmLirStmt::ConstI32 { value: 65_535, .. })),
        "U16 maximum must keep its bit pattern through the U16-to-U32 passthrough"
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
        "same-carrier U16-to-U32 must not emit a conversion"
    );
}

#[test]
fn uint_to_string_formats_through_the_unsigned_helper() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();

    let cases = [
        (FixedScalar::U32, 4_294_967_295, WasmAbiType::I32),
        (
            FixedScalar::U64,
            18_446_744_073_709_551_615,
            WasmAbiType::I64,
        ),
    ];
    for (scalar, value, carrier) in cases {
        let mut expressions = HirExpressionStore::default();
        let source = uint_literal(&mut expressions, scalar, value);
        let cast = expression(
            HirExpressionKind::Cast {
                source,
                policy: BuiltinCastPolicyId::NumericToString(NumericScalar::Fixed(scalar)),
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
            &mut path_fork,
            &mut string_table,
            &type_environment,
        );
        assert!(
            function.blocks[0]
                .statements
                .iter()
                .any(|statement| matches!(statement, WasmLirStmt::StringFromU64 { .. })),
            "{scalar:?} formatting must use the unsigned text helper"
        );
        assert!(
            !function.blocks[0]
                .statements
                .iter()
                .any(|statement| matches!(statement, WasmLirStmt::StringFromI64 { .. })),
            "{scalar:?} formatting must not use the signed text helper"
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
    for (scalar, expected_kind) in [
        (FixedScalar::U32, WasmIntegerOperationKind::Unsigned32),
        (FixedScalar::U64, WasmIntegerOperationKind::Unsigned64),
    ] {
        for operator in operators {
            let mut expressions = HirExpressionStore::default();
            let function_path = path_fork
                .try_intern_portable_path("operate", &mut string_table)
                .expect("test path fits");
            let operands = HirNumericOperands::Binary {
                left: uint_literal(&mut expressions, scalar, 7),
                right: uint_literal(&mut expressions, scalar, 3),
            };
            let block = HirBlock {
                id: BlockId(0),
                region: RegionId(0),
                locals: vec![local(0, fixed_scalar_type(scalar), RegionId(0))],
                statements: vec![statement(
                    902,
                    HirStatementKind::NumericOp {
                        op: HirNumericOp {
                            operator,
                            domain: NumericScalar::Fixed(scalar),
                        },
                        failure_mode: NumericFailureMode::Trap,
                        operands,
                        result: HirLocalDestination::Define(LocalId(0)),
                    },
                    902,
                )],
                terminator: HirTerminator::Return({
                    let value = load_local(
                        &mut expressions,
                        LocalId(0),
                        fixed_scalar_type(scalar),
                        RegionId(0),
                    );
                    expressions
                        .append_values(&[value], None)
                        .expect("fixture return values should fit")
                }),
            };
            let function = HirFunction {
                id: FunctionId(0),
                entry: BlockId(0),
                params: vec![],
                returns: vec![fixed_scalar_type(scalar)].into_boxed_slice(),
                error_return: None,
            };
            let module = build_module(
                &mut path_fork,
                &mut string_table,
                expressions,
                vec![(function, function_path, HirFunctionOrigin::Normal)],
                vec![block],
                FunctionId(0),
            );
            let request = WasmBackendRequest::default();
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
                panic!("unsigned integer {operator:?} should lower with {scalar:?}: {error:?}")
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
                "{scalar:?} unsigned integer {operator:?} must lower as {expected_kind:?}"
            );
        }
    }
}
