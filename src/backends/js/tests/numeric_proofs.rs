//! Executable parity tests for the numeric proof table consumer.
//!
//! WHAT: lowers the same hand-built HIR twice — once with the analysed proof table and once with
//!       the empty default table that retains every check — and executes both generated modules
//!       in Node.js.
//! WHY: proven integer operations and narrowings must produce equal exact results, error codes
//!       and effect ordering while omitting only the checked machinery the proof makes
//!       unreachable, including signed-zero normalization and required ABI result shapes.

use super::support::*;
use crate::compiler_frontend::analysis::numeric_proofs::analyse_numeric_proofs;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId, LocalId, RegionId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::hir::validate_hir_module;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use std::process::Command;

/// A hand-built single-block numeric fixture body.
struct NumericFixture {
    locals: Vec<HirLocal>,
    local_names: Vec<(LocalId, &'static str)>,
    statements: Vec<HirStatement>,
    return_value: crate::compiler_frontend::hir::ids::HirValueId,
}

/// Both generated modules for one fixture plus the analysed table and emitted function name.
struct LoweredFixture {
    analysed_source: String,
    retained_source: String,
    analysed_table: NumericProofs,
    function_name: String,
}

/// Builds, analyses and lowers one single-function fixture under both proof tables.
fn lower_numeric_fixture(
    function_name: &str,
    build: impl FnOnce(
        &TypeIds,
        &mut TypeEnvironment,
        RegionId,
        &mut HirExpressionStore,
    ) -> NumericFixture,
) -> LoweredFixture {
    let (mut type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let mut expressions = HirExpressionStore::default();
    let fixture = build(&types, &mut type_environment, region, &mut expressions);
    let return_type = expressions.expression(fixture.return_value).ty;
    let returned = append_values(&[fixture.return_value], &mut expressions);

    lower_numeric_blocks(
        function_name,
        expressions,
        type_environment,
        vec![HirBlock {
            id: BlockId(0),
            region,
            locals: fixture.locals,
            statements: fixture.statements,
            terminator: HirTerminator::Return(returned),
        }],
        HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![],
            returns: vec![return_type].into_boxed_slice(),
            error_return: None,
        },
        fixture.local_names,
    )
}

fn lower_numeric_blocks(
    function_name: &str,
    expressions: HirExpressionStore,
    type_environment: TypeEnvironment,
    blocks: Vec<HirBlock>,
    function: HirFunction,
    local_names: Vec<(LocalId, &'static str)>,
) -> LoweredFixture {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    // Hand-built fixtures carry the same entry-start function-origin tag production lowering
    // records, so the fixture shape matches what HIR validation accepts.
    let mut module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        function_name,
        blocks,
        function,
        &local_names,
    );
    module
        .function_origins
        .insert(FunctionId(0), HirFunctionOrigin::EntryStart);
    validate_hir_module(&module, &type_environment, &ExternalPackageRegistry::new())
        .expect("numeric fixture HIR must satisfy production HIR validation");

    let analysed_table = analyse_numeric_proofs(&module, &type_environment);
    let config = JsLoweringConfig::direct_js(false);
    let analysed_module = lower_hir_to_js(
        &module,
        &analysed_table,
        &string_table,
        config.clone(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("analysed-table lowering should succeed");
    let retained_module = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        config,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("retained-table lowering should succeed");

    LoweredFixture {
        function_name: analysed_module
            .function_name_by_id
            .get(&FunctionId(0))
            .cloned()
            .expect("emitted module carries its function name"),
        analysed_source: analysed_module.source,
        retained_source: retained_module.source,
        analysed_table,
    }
}

fn named_locals(
    specs: &[(u32, TypeId, &'static str)],
    region: RegionId,
) -> (Vec<HirLocal>, Vec<(LocalId, &'static str)>) {
    let locals = specs
        .iter()
        .map(|(id, ty, _)| local(*id, *ty, region))
        .collect();
    let names = specs
        .iter()
        .map(|(id, _, name)| (LocalId(*id), *name))
        .collect();
    (locals, names)
}

fn fixed_const(
    value: FixedScalarValue,
    ty: TypeId,
    region: RegionId,
    expressions: &mut HirExpressionStore,
) -> crate::compiler_frontend::hir::ids::HirValueId {
    expression(
        HirExpressionKind::FixedScalar(value),
        ty,
        region,
        ValueKind::Const,
        expressions,
    )
}

fn load_local(
    target: u32,
    ty: TypeId,
    region: RegionId,
    expressions: &mut HirExpressionStore,
) -> crate::compiler_frontend::hir::ids::HirValueId {
    expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(target))),
        ty,
        region,
        ValueKind::RValue,
        expressions,
    )
}

fn define_local(
    target: u32,
    value: crate::compiler_frontend::hir::ids::HirValueId,
) -> HirStatementKind {
    HirStatementKind::Write {
        target: HirWriteTarget::DefineLocal(LocalId(target)),
        value,
    }
}

fn binary_operands(
    left: crate::compiler_frontend::hir::ids::HirValueId,
    right: crate::compiler_frontend::hir::ids::HirValueId,
) -> HirNumericOperands {
    HirNumericOperands::Binary { left, right }
}

fn trap_op(
    operator: NumericOperator,
    domain: NumericScalar,
    operands: HirNumericOperands,
    result: u32,
) -> HirStatementKind {
    HirStatementKind::NumericOp {
        op: HirNumericOp { operator, domain },
        failure_mode: NumericFailureMode::Trap,
        operands,
        result: HirLocalDestination::Define(LocalId(result)),
    }
}

fn narrowing_cast(
    source_scalar: FixedScalar,
    target_scalar: FixedScalar,
    source: crate::compiler_frontend::hir::ids::HirValueId,
) -> HirFallibleOperation {
    HirFallibleOperation::CastOp {
        policy: BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(source_scalar),
            target: NumericScalar::Fixed(target_scalar),
        },
        source,
    }
}

struct NodeRun {
    stdout: String,
    stderr: String,
    success: bool,
}

fn run_javascript(source: &str) -> NodeRun {
    let output = Command::new("node")
        .args(["--eval", source])
        .output()
        .expect("Node.js is required for JavaScript runtime behavior tests");
    NodeRun {
        stdout: String::from_utf8(output.stdout).expect("Node.js output is UTF-8"),
        stderr: String::from_utf8(output.stderr).expect("Node.js output is UTF-8"),
        success: output.status.success(),
    }
}

fn run_both_tables(lowered: &LoweredFixture, driver: &str) -> (NodeRun, NodeRun) {
    (
        run_javascript(&format!("{}\n{}", lowered.analysed_source, driver)),
        run_javascript(&format!("{}\n{}", lowered.retained_source, driver)),
    )
}

fn assert_same_runtime(analysed: &NodeRun, retained: &NodeRun, context: &str) {
    assert_eq!(
        analysed.stdout, retained.stdout,
        "{context}: runtime output diverged between the analysed and retained proof tables"
    );
    assert_eq!(
        analysed.stderr, retained.stderr,
        "{context}: runtime error output diverged between the analysed and retained proof tables"
    );
    assert_eq!(
        analysed.success, retained.success,
        "{context}: exit status diverged between the analysed and retained proof tables"
    );
}

/// Verifies that a fully proven operation chain omits the whole checked helper family and still
/// produces the exact retained-table result at runtime.
#[test]
fn proven_operation_chain_matches_retained_runtime_without_helper_family() {
    let lowered = lower_numeric_fixture(
        "proven_chain",
        |types, _environment, region, expressions| {
            let (locals, local_names) = named_locals(
                &[
                    (0, types.i32, "a"),
                    (1, types.i32, "b"),
                    (2, types.i32, "c"),
                    (3, types.i32, "d"),
                    (4, types.i32, "e"),
                    (5, types.i32, "f"),
                ],
                region,
            );
            let int = types.i32;
            NumericFixture {
                locals,
                local_names,
                statements: vec![
                    statement(
                        1,
                        define_local(0, i32_expression(2, int, region, expressions)),
                    ),
                    statement(
                        2,
                        trap_op(
                            NumericOperator::Add,
                            NumericScalar::Fixed(FixedScalar::I32),
                            binary_operands(
                                load_local(0, int, region, expressions),
                                i32_expression(3, int, region, expressions),
                            ),
                            1,
                        ),
                    ),
                    statement(
                        3,
                        trap_op(
                            NumericOperator::Multiply,
                            NumericScalar::Fixed(FixedScalar::I32),
                            binary_operands(
                                load_local(1, int, region, expressions),
                                i32_expression(4, int, region, expressions),
                            ),
                            2,
                        ),
                    ),
                    statement(
                        4,
                        trap_op(
                            NumericOperator::Subtract,
                            NumericScalar::Fixed(FixedScalar::I32),
                            binary_operands(
                                load_local(2, int, region, expressions),
                                i32_expression(5, int, region, expressions),
                            ),
                            3,
                        ),
                    ),
                    statement(
                        5,
                        trap_op(
                            NumericOperator::IntegerDivide,
                            NumericScalar::Fixed(FixedScalar::I32),
                            binary_operands(
                                load_local(3, int, region, expressions),
                                i32_expression(2, int, region, expressions),
                            ),
                            4,
                        ),
                    ),
                    statement(
                        6,
                        trap_op(
                            NumericOperator::Remainder,
                            NumericScalar::Fixed(FixedScalar::I32),
                            binary_operands(
                                load_local(4, int, region, expressions),
                                i32_expression(4, int, region, expressions),
                            ),
                            5,
                        ),
                    ),
                ],
                return_value: load_local(5, int, region, expressions),
            }
        },
    );

    // Statements 2..=6 are the five chained operations; statement 1 is the literal seed.
    for statement_id in 2..=6 {
        assert!(
            lowered
                .analysed_table
                .integer_operation_is_safe(HirNodeId(statement_id)),
            "chain operation {statement_id} must be proven safe"
        );
    }

    for helper in [
        "__moth_int_add(",
        "__moth_int_sub(",
        "__moth_int_mul(",
        "__moth_int_div(",
        "__moth_int_mod(",
        "__moth_int_check(",
        "__moth_int_ok(",
        "__moth_numeric_trap(",
    ] {
        assert!(
            !lowered.analysed_source.contains(helper),
            "fully proven chains must not reference or emit {helper}"
        );
    }
    assert!(
        lowered.retained_source.contains("function __moth_int_add("),
        "the empty proof table must keep the checked helper family"
    );

    let driver = format!("console.log({}());", lowered.function_name);
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(&analysed, &retained, "proven operation chain");
    assert_eq!(analysed.stdout.trim(), "3");
}

/// Runs one proven zero-producing operation through the signed-zero probe in both tables.
fn assert_proven_zero_normalization(
    name: &str,
    build_operation: impl FnOnce(
        &TypeIds,
        &mut TypeEnvironment,
        RegionId,
        &mut HirExpressionStore,
    ) -> (HirStatementKind, TypeId),
) {
    let lowered = lower_numeric_fixture(name, |types, environment, region, expressions| {
        let (operation, destination) = build_operation(types, environment, region, expressions);
        let (locals, local_names) = named_locals(&[(0, destination, "v")], region);
        NumericFixture {
            locals,
            local_names,
            statements: vec![statement(1, operation)],
            return_value: load_local(0, destination, region, expressions),
        }
    });

    assert!(
        lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(1)),
        "{name} must be proven safe"
    );

    let driver = format!(
        "const value = {}();\nconsole.log(1 / value, Object.is(value, -0), value === 0);",
        lowered.function_name
    );
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(&analysed, &retained, name);
    assert_eq!(
        analysed.stdout.trim(),
        "Infinity false true",
        "{name} must normalize JS -0 to the single Moth zero in both tables"
    );
}

/// Verifies that every Number-carrier operation able to produce JS `-0` normalizes it exactly
/// like the checked helpers' success boundary.
#[test]
fn proven_signed_zero_normalization_matches_retained_checks() {
    assert_proven_zero_normalization("zero_mul", |types, _environment, region, expressions| {
        (
            trap_op(
                NumericOperator::Multiply,
                NumericScalar::Fixed(FixedScalar::I32),
                binary_operands(
                    i32_expression(0, types.i32, region, expressions),
                    i32_expression(-5, types.i32, region, expressions),
                ),
                0,
            ),
            types.i32,
        )
    });
    assert_proven_zero_normalization(
        "zero_mul_negative_times_zero",
        |_types, _environment, region, expressions| {
            // Fixed narrow integers promote to their common I32 computation domain in validated
            // HIR, so the negative-times-zero fixture multiplies I32 literals.
            let i32_ty = builtin_type_ids::fixed_scalar(FixedScalar::I32);
            (
                trap_op(
                    NumericOperator::Multiply,
                    NumericScalar::Fixed(FixedScalar::I32),
                    binary_operands(
                        fixed_const(
                            FixedScalarValue::signed(FixedScalar::I32, -128)
                                .expect("-128 fits I32"),
                            i32_ty,
                            region,
                            expressions,
                        ),
                        fixed_const(
                            FixedScalarValue::signed(FixedScalar::I32, 0).expect("0 fits I32"),
                            i32_ty,
                            region,
                            expressions,
                        ),
                    ),
                    0,
                ),
                i32_ty,
            )
        },
    );
    assert_proven_zero_normalization("zero_neg", |types, _environment, region, expressions| {
        (
            HirStatementKind::NumericOp {
                op: HirNumericOp {
                    operator: NumericOperator::Negate,
                    domain: NumericScalar::Fixed(FixedScalar::I32),
                },
                failure_mode: NumericFailureMode::Trap,
                operands: HirNumericOperands::Unary {
                    operand: i32_expression(0, types.i32, region, expressions),
                },
                result: HirLocalDestination::Define(LocalId(0)),
            },
            types.i32,
        )
    });
    assert_proven_zero_normalization("zero_div", |types, _environment, region, expressions| {
        (
            trap_op(
                NumericOperator::IntegerDivide,
                NumericScalar::Fixed(FixedScalar::I32),
                binary_operands(
                    i32_expression(0, types.i32, region, expressions),
                    i32_expression(-3, types.i32, region, expressions),
                ),
                0,
            ),
            types.i32,
        )
    });
    assert_proven_zero_normalization("zero_mod", |types, _environment, region, expressions| {
        (
            trap_op(
                NumericOperator::Remainder,
                NumericScalar::Fixed(FixedScalar::I32),
                binary_operands(
                    i32_expression(-7, types.i32, region, expressions),
                    i32_expression(7, types.i32, region, expressions),
                ),
                0,
            ),
            types.i32,
        )
    });
}

/// Verifies that a proven I32 negation of the negative literal `-5` executes in both
/// proof tables and returns the exact positive literal `5` (a fused `--` literal must fail).
#[test]
fn proven_negative_literal_negation_matches_positive_number_literal() {
    let lowered = lower_numeric_fixture(
        "negative_literal_negation_number",
        |types, _environment, region, expressions| {
            let int = types.i32;
            let (locals, local_names) = named_locals(&[(0, int, "v")], region);
            NumericFixture {
                locals,
                local_names,
                statements: vec![statement(
                    1,
                    HirStatementKind::NumericOp {
                        op: HirNumericOp {
                            operator: NumericOperator::Negate,
                            domain: NumericScalar::Fixed(FixedScalar::I32),
                        },
                        failure_mode: NumericFailureMode::Trap,
                        operands: HirNumericOperands::Unary {
                            operand: i32_expression(-5, int, region, expressions),
                        },
                        result: HirLocalDestination::Define(LocalId(0)),
                    },
                )],
                return_value: load_local(0, int, region, expressions),
            }
        },
    );

    assert!(
        lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(1)),
        "negating -5 stays inside the I32 domain and must be proven"
    );

    let driver = format!("console.log({}());", lowered.function_name);
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(&analysed, &retained, "negative literal negation (Number)");
    assert!(
        analysed.success,
        "negating a negative I32 literal must execute, not fail to parse: {}",
        analysed.stderr
    );
    assert_eq!(
        analysed.stdout.trim(),
        "5",
        "the proven negation of -5 must equal the exact positive literal 5"
    );
}

/// Verifies that a proven I64 BigInt negation of the negative literal `-5n` executes in both
/// proof tables and returns the exact positive literal `5n` (a fused `--` literal must fail).
#[test]
fn proven_negative_literal_negation_matches_positive_bigint_literal() {
    let lowered = lower_numeric_fixture(
        "negative_literal_negation_bigint",
        |_types, _environment, region, expressions| {
            let i64_ty = builtin_type_ids::fixed_scalar(FixedScalar::I64);
            let (locals, local_names) = named_locals(&[(0, i64_ty, "v")], region);
            NumericFixture {
                locals,
                local_names,
                statements: vec![statement(
                    1,
                    HirStatementKind::NumericOp {
                        op: HirNumericOp {
                            operator: NumericOperator::Negate,
                            domain: NumericScalar::Fixed(FixedScalar::I64),
                        },
                        failure_mode: NumericFailureMode::Trap,
                        operands: HirNumericOperands::Unary {
                            operand: fixed_const(
                                FixedScalarValue::signed(FixedScalar::I64, -5)
                                    .expect("-5 fits I64"),
                                i64_ty,
                                region,
                                expressions,
                            ),
                        },
                        result: HirLocalDestination::Define(LocalId(0)),
                    },
                )],
                return_value: load_local(0, i64_ty, region, expressions),
            }
        },
    );

    assert!(
        lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(1)),
        "negating -5n stays inside I64 and must be proven"
    );

    let driver = format!("console.log({}());", lowered.function_name);
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(&analysed, &retained, "negative literal negation (BigInt)");
    assert!(
        analysed.success,
        "negating a negative BigInt literal must execute, not fail to parse: {}",
        analysed.stderr
    );
    assert_eq!(
        analysed.stdout.trim(),
        "5n",
        "the proven negation of -5n must equal the exact positive BigInt literal 5n"
    );
}

/// Verifies that operations the analysis declines keep their checks and produce identical
/// runtime error codes and output in both tables.
#[test]
fn unproven_operations_keep_runtime_errors_identical_across_tables() {
    let overflow = lower_numeric_fixture(
        "u32_overflow",
        |_types, _environment, region, expressions| {
            // Fixed narrow integers promote to their common U32 computation domain in validated
            // HIR, so the unsigned overflow fixture multiplies U32 literals past U32::MAX.
            let u32_ty = builtin_type_ids::fixed_scalar(FixedScalar::U32);
            let (locals, local_names) = named_locals(&[(0, u32_ty, "v")], region);
            NumericFixture {
                locals,
                local_names,
                statements: vec![statement(
                    1,
                    trap_op(
                        NumericOperator::Multiply,
                        NumericScalar::Fixed(FixedScalar::U32),
                        binary_operands(
                            fixed_const(
                                FixedScalarValue::unsigned(FixedScalar::U32, 4_000_000_000)
                                    .expect("4000000000 fits U32"),
                                u32_ty,
                                region,
                                expressions,
                            ),
                            fixed_const(
                                FixedScalarValue::unsigned(FixedScalar::U32, 2)
                                    .expect("2 fits U32"),
                                u32_ty,
                                region,
                                expressions,
                            ),
                        ),
                        0,
                    ),
                )],
                return_value: load_local(0, u32_ty, region, expressions),
            }
        },
    );
    assert!(
        !overflow
            .analysed_table
            .integer_operation_is_safe(HirNodeId(1)),
        "an overflowing product must not be proven"
    );
    assert!(
        overflow
            .analysed_source
            .contains("function __moth_int_mul("),
        "an unproven operation must keep its checked helper"
    );
    let driver = format!("{}();", overflow.function_name);
    let (analysed, retained) = run_both_tables(&overflow, &driver);
    assert_same_runtime(&analysed, &retained, "U32 overflow");
    assert!(!analysed.success, "the U32 overflow must fail at runtime");
    let overflow_message = BuiltinErrorCode::IntOverflow.default_message();
    assert!(
        analysed.stderr.contains(overflow_message),
        "the U32 overflow must surface the canonical IntOverflow message: {}",
        analysed.stderr
    );

    let divide_by_zero = lower_numeric_fixture(
        "divide_by_zero",
        |types, _environment, region, expressions| {
            let (locals, local_names) = named_locals(&[(0, types.i32, "v")], region);
            NumericFixture {
                locals,
                local_names,
                statements: vec![statement(
                    1,
                    trap_op(
                        NumericOperator::IntegerDivide,
                        NumericScalar::Fixed(FixedScalar::I32),
                        binary_operands(
                            i32_expression(5, types.i32, region, expressions),
                            i32_expression(0, types.i32, region, expressions),
                        ),
                        0,
                    ),
                )],
                return_value: load_local(0, types.i32, region, expressions),
            }
        },
    );
    assert!(
        !divide_by_zero
            .analysed_table
            .integer_operation_is_safe(HirNodeId(1)),
        "a zero-valued divisor interval must not be proven"
    );
    assert!(
        divide_by_zero
            .analysed_source
            .contains("function __moth_int_div("),
        "a zero-divisor operation must keep its checked helper"
    );
    let driver = format!("{}();", divide_by_zero.function_name);
    let (analysed, retained) = run_both_tables(&divide_by_zero, &driver);
    assert_same_runtime(&analysed, &retained, "divide by zero");
    assert!(!analysed.success, "division by zero must fail at runtime");
    let divide_message = BuiltinErrorCode::DivideByZero.default_message();
    assert!(
        analysed.stderr.contains(divide_message),
        "division by zero must surface the canonical DivideByZero message: {}",
        analysed.stderr
    );
}

/// Verifies that `Power` keeps its checked lowering and trap machinery even when operands are
/// exact literals.
#[test]
fn power_always_retains_checked_lowering() {
    let lowered = lower_numeric_fixture(
        "power_checked",
        |types, _environment, region, expressions| {
            let (locals, local_names) = named_locals(&[(0, types.i32, "v")], region);
            NumericFixture {
                locals,
                local_names,
                statements: vec![statement(
                    1,
                    trap_op(
                        NumericOperator::Power,
                        NumericScalar::Fixed(FixedScalar::I32),
                        binary_operands(
                            i32_expression(2, types.i32, region, expressions),
                            i32_expression(3, types.i32, region, expressions),
                        ),
                        0,
                    ),
                )],
                return_value: load_local(0, types.i32, region, expressions),
            }
        },
    );

    assert!(
        !lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(1)),
        "Power must never be proven"
    );
    assert!(
        lowered.analysed_source.contains("__moth_int_pow("),
        "the analysed table must keep the checked Power helper demanded by its call"
    );
    assert!(
        lowered
            .analysed_source
            .contains("function __moth_numeric_trap("),
        "the analysed table must keep the trap machinery for Power"
    );

    let driver = format!("console.log({}());", lowered.function_name);
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(&analysed, &retained, "checked power");
    assert_eq!(analysed.stdout.trim(), "8");
}

/// A native Invoke chain returns its final scalar through the fallible ABI. The sibling fixtures
/// recover the deliberately rejected second operation into separate success and error slots.
fn lower_recoverable_fixture(
    name: &str,
    build: impl FnOnce(
        &TypeIds,
        RegionId,
        &mut HirExpressionStore,
    ) -> Vec<(u32, HirFallibleOperation, TypeId)>,
) -> LoweredFixture {
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let mut expressions = HirExpressionStore::default();
    let operations = build(&types, region, &mut expressions);
    let count = operations.len();
    let result_type = operations[0].2;
    let mut blocks = Vec::new();
    let mut locals = Vec::new();
    let mut names = Vec::new();
    for (index, (id, operation, ty)) in operations.into_iter().enumerate() {
        let result = LocalId(index as u32);
        let error = LocalId((count + index) as u32);
        locals.push(local(result.0, ty, region));
        locals.push(local(error.0, types.error, region));
        names.push((result, "result"));
        names.push((error, "error"));
        blocks.push(HirBlock {
            id: BlockId(index as u32),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Invoke {
                id: HirNodeId(id),
                span: None,
                operation,
                success: BlockId((index + 1) as u32),
                success_results: vec![result],
                failure: BlockId((count + 1 + index) as u32),
                error_result: error,
            },
        });
    }
    blocks[0].locals = locals;
    blocks.push(HirBlock {
        id: BlockId(count as u32),
        region,
        locals: vec![],
        statements: vec![],
        terminator: if count == 1 {
            let returned = load_local(0, result_type, region, &mut expressions);
            HirTerminator::Return(append_values(&[returned], &mut expressions))
        } else {
            HirTerminator::RuntimeFailure {
                message: "the deliberately rejected sibling unexpectedly succeeded".to_owned(),
                cause: None,
            }
        },
    });
    for index in 0..count {
        let error = load_local(
            (count + index) as u32,
            types.error,
            region,
            &mut expressions,
        );
        blocks.push(HirBlock {
            id: BlockId((count + 1 + index) as u32),
            region,
            locals: vec![],
            statements: vec![],
            terminator: if count > 1 && index == count - 1 {
                let returned = load_local(0, result_type, region, &mut expressions);
                HirTerminator::Return(append_values(&[returned, error], &mut expressions))
            } else {
                HirTerminator::ReturnError(error)
            },
        });
    }
    let returns = if count == 1 {
        vec![result_type]
    } else {
        vec![result_type, types.error]
    };
    lower_numeric_blocks(
        name,
        expressions,
        type_environment,
        blocks,
        HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![],
            returns: returns.into_boxed_slice(),
            error_return: Some(types.error),
        },
        names,
    )
}

#[test]
fn proven_invoke_elides_checks_and_preserves_success_abi() {
    let lowered = lower_recoverable_fixture("proven_invoke", |types, region, expressions| {
        vec![(
            2,
            HirFallibleOperation::NumericOp {
                op: HirNumericOp {
                    operator: NumericOperator::Add,
                    domain: NumericScalar::Fixed(FixedScalar::I32),
                },
                operands: binary_operands(
                    i32_expression(2, types.i32, region, expressions),
                    i32_expression(3, types.i32, region, expressions),
                ),
            },
            types.i32,
        )]
    });
    assert!(
        lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(2))
    );
    assert!(!lowered.analysed_source.contains("function __moth_int_add("));
    assert!(!lowered.analysed_source.contains("__moth_int_check("));
    assert!(!lowered.analysed_source.contains("__moth_numeric_trap("));
    let driver = format!("console.log(JSON.stringify({}()));", lowered.function_name);
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(&analysed, &retained, "proven Invoke success ABI");
    assert_eq!(analysed.stdout.trim(), r#"{"tag":"ok","value":5}"#);
}

#[test]
fn proven_invoke_with_unsafe_sibling_keeps_checked_family() {
    let lowered = lower_recoverable_fixture("invoke_sibling", |types, region, expressions| {
        [2, i32::MAX]
            .into_iter()
            .enumerate()
            .map(|(index, left)| {
                (
                    (2 + index * 2) as u32,
                    HirFallibleOperation::NumericOp {
                        op: HirNumericOp {
                            operator: NumericOperator::Add,
                            domain: NumericScalar::Fixed(FixedScalar::I32),
                        },
                        operands: binary_operands(
                            i32_expression(left, types.i32, region, expressions),
                            i32_expression(3, types.i32, region, expressions),
                        ),
                    },
                    types.i32,
                )
            })
            .collect()
    });
    assert!(
        lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(2))
    );
    assert!(
        !lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(4))
    );
    assert!(lowered.analysed_source.contains("function __moth_int_add("));
    let driver = format!(
        "const results = {}().value;\nconsole.log(results[0], __moth_error_code(results[1]));",
        lowered.function_name,
    );
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(&analysed, &retained, "Invoke sibling recovery");
    assert_eq!(
        analysed.stdout.trim(),
        format!("5 {}", BuiltinErrorCode::IntOverflow.as_u32())
    );
}

/// Narrowing elides only its predicate, preserving physical conversion and the fallible ABI.
#[test]
fn proven_narrowing_elides_checked_cast_helpers() {
    for (source, target, value) in [
        (FixedScalar::I32, FixedScalar::I8, 100),
        (FixedScalar::I64, FixedScalar::I32, 1000),
    ] {
        let lowered = lower_recoverable_fixture("proven_narrow", |_types, region, expressions| {
            let source_type = builtin_type_ids::fixed_scalar(source);
            let target_type = builtin_type_ids::fixed_scalar(target);
            vec![(
                1,
                narrowing_cast(
                    source,
                    target,
                    fixed_const(
                        FixedScalarValue::signed(source, value)
                            .expect("fixture value fits its source"),
                        source_type,
                        region,
                        expressions,
                    ),
                ),
                target_type,
            )]
        });
        assert!(
            lowered
                .analysed_table
                .integer_narrowing_is_safe(HirNodeId(1))
        );
        assert!(
            !lowered
                .analysed_source
                .contains("function __moth_cast_integer_to_integer(")
        );
        assert!(
            !lowered
                .analysed_source
                .contains("__moth_cast_integer_in_range")
        );
        assert!(
            lowered
                .retained_source
                .contains("function __moth_cast_integer_to_integer(")
        );
        let driver = format!("console.log(JSON.stringify({}()));", lowered.function_name);
        let (analysed, retained) = run_both_tables(&lowered, &driver);
        assert_same_runtime(&analysed, &retained, "proven narrowing ABI");
        assert_eq!(
            analysed.stdout.trim(),
            format!("{{\"tag\":\"ok\",\"value\":{value}}}")
        );
    }
}

#[test]
fn narrowing_policy_with_retained_use_keeps_checked_helpers() {
    let lowered = lower_recoverable_fixture("shared_narrow", |_types, region, expressions| {
        let source_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
        let target_type = builtin_type_ids::fixed_scalar(FixedScalar::I8);
        [100, 200]
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                (
                    (index + 1) as u32,
                    narrowing_cast(
                        FixedScalar::I32,
                        FixedScalar::I8,
                        fixed_const(
                            FixedScalarValue::signed(FixedScalar::I32, value)
                                .expect("fixture value fits I32"),
                            source_type,
                            region,
                            expressions,
                        ),
                    ),
                    target_type,
                )
            })
            .collect()
    });
    assert!(
        lowered
            .analysed_table
            .integer_narrowing_is_safe(HirNodeId(1))
    );
    assert!(
        !lowered
            .analysed_table
            .integer_narrowing_is_safe(HirNodeId(2))
    );
    assert!(
        lowered
            .analysed_source
            .contains("function __moth_cast_integer_to_integer(")
    );
    let driver = format!(
        "const results = {}().value;\nconsole.log(JSON.stringify({{tag: \"ok\", value: results[0]}}), __moth_error_code(results[1]));",
        lowered.function_name,
    );
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(&analysed, &retained, "shared narrowing policy");
    assert_eq!(
        analysed.stdout.trim(),
        format!(
            "{{\"tag\":\"ok\",\"value\":100}} {}",
            BuiltinErrorCode::IntCastOutOfRange.as_u32(),
        )
    );
}

/// Verifies that exact U64 remainders stay on the BigInt carrier when proven.
#[test]
fn proven_u64_remainder_stays_on_bigint_carrier() {
    let lowered = lower_numeric_fixture(
        "u64_remainder",
        |_types, _environment, region, expressions| {
            let u64_ty = builtin_type_ids::fixed_scalar(FixedScalar::U64);
            let (locals, local_names) = named_locals(&[(0, u64_ty, "v")], region);
            NumericFixture {
                locals,
                local_names,
                statements: vec![statement(
                    1,
                    trap_op(
                        NumericOperator::Remainder,
                        NumericScalar::Fixed(FixedScalar::U64),
                        binary_operands(
                            fixed_const(
                                FixedScalarValue::unsigned(FixedScalar::U64, 7)
                                    .expect("7 fits U64"),
                                u64_ty,
                                region,
                                expressions,
                            ),
                            fixed_const(
                                FixedScalarValue::unsigned(FixedScalar::U64, 3)
                                    .expect("3 fits U64"),
                                u64_ty,
                                region,
                                expressions,
                            ),
                        ),
                        0,
                    ),
                )],
                return_value: load_local(0, u64_ty, region, expressions),
            }
        },
    );

    assert!(
        lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(1)),
        "an exact U64 remainder with a zero-free divisor must be proven"
    );
    assert!(
        !lowered.analysed_source.contains("__moth_bigint_mod("),
        "a proven U64 remainder must not reference the checked BigInt helper"
    );
    assert!(
        lowered
            .retained_source
            .contains("function __moth_bigint_mod("),
        "the empty proof table must keep the checked BigInt helper family"
    );

    let driver = format!("console.log({}());", lowered.function_name);
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(&analysed, &retained, "proven U64 remainder");
    assert_eq!(analysed.stdout.trim(), "1n");
}

/// Verifies that proven lowering preserves the exact runtime result of a sequential numeric chain.
#[test]
fn proven_lowering_preserves_numeric_statement_order() {
    let lowered = lower_numeric_fixture(
        "proven_numeric_chain",
        |types, _environment, region, expressions| {
            let (locals, local_names) = named_locals(
                &[
                    (0, types.i32, "a"),
                    (1, types.i32, "b"),
                    (2, types.i32, "c"),
                    (3, types.i32, "d"),
                    (4, types.i32, "e"),
                    (5, types.i32, "f"),
                ],
                region,
            );
            let int = types.i32;
            NumericFixture {
                locals,
                local_names,
                statements: vec![
                    statement(
                        1,
                        define_local(0, i32_expression(2, int, region, expressions)),
                    ),
                    statement(
                        2,
                        trap_op(
                            NumericOperator::Add,
                            NumericScalar::Fixed(FixedScalar::I32),
                            binary_operands(
                                load_local(0, int, region, expressions),
                                i32_expression(3, int, region, expressions),
                            ),
                            1,
                        ),
                    ),
                    statement(
                        3,
                        trap_op(
                            NumericOperator::Multiply,
                            NumericScalar::Fixed(FixedScalar::I32),
                            binary_operands(
                                load_local(1, int, region, expressions),
                                i32_expression(4, int, region, expressions),
                            ),
                            2,
                        ),
                    ),
                    statement(
                        4,
                        trap_op(
                            NumericOperator::Subtract,
                            NumericScalar::Fixed(FixedScalar::I32),
                            binary_operands(
                                load_local(2, int, region, expressions),
                                i32_expression(5, int, region, expressions),
                            ),
                            3,
                        ),
                    ),
                    statement(
                        5,
                        trap_op(
                            NumericOperator::IntegerDivide,
                            NumericScalar::Fixed(FixedScalar::I32),
                            binary_operands(
                                load_local(3, int, region, expressions),
                                i32_expression(2, int, region, expressions),
                            ),
                            4,
                        ),
                    ),
                    statement(
                        6,
                        trap_op(
                            NumericOperator::Remainder,
                            NumericScalar::Fixed(FixedScalar::I32),
                            binary_operands(
                                load_local(4, int, region, expressions),
                                i32_expression(4, int, region, expressions),
                            ),
                            5,
                        ),
                    ),
                ],
                return_value: load_local(5, int, region, expressions),
            }
        },
    );

    let driver = format!("console.log({}());", lowered.function_name);
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(&analysed, &retained, "proven numeric chain");
    assert_eq!(analysed.stdout.trim(), "3");
}
