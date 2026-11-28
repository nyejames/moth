//! Executable parity tests for the numeric proof table consumer.
//!
//! WHAT: lowers the same hand-built HIR twice — once with the analysed proof table, which derives
//!       sole-success-edge seeds, and once with the empty default table that retains every check —
//!       and executes both generated modules in Node.js.
//! WHY: proven integer operations and narrowings must produce equal exact results, error codes
//!       and effect ordering while omitting only the checked machinery the proof makes
//!       unreachable, including signed-zero normalization and required ABI result shapes.

use super::support::*;
use crate::compiler_frontend::analysis::numeric_proofs::analyse_numeric_proofs;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::CallableReturnShape;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionReturnContract};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId, LocalId, RegionId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{
    HirFallibleOperation, HirTerminator, RuntimeFailureKind,
};
use crate::compiler_frontend::hir::validate_hir_module;
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::hir_fixture_support::lower_hir_result;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
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
            return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
                vec![return_type],
                None,
            )),
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
    let module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        function_name,
        blocks,
        function,
        &local_names,
    );
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
        op: HirNumericOp::ordinary(operator, domain),
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
                op: HirNumericOp::ordinary(
                    NumericOperator::Negate,
                    NumericScalar::Fixed(FixedScalar::I32),
                ),
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
                        op: HirNumericOp::ordinary(
                            NumericOperator::Negate,
                            NumericScalar::Fixed(FixedScalar::I32),
                        ),
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
                        op: HirNumericOp::ordinary(
                            NumericOperator::Negate,
                            NumericScalar::Fixed(FixedScalar::I64),
                        ),
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

/// A source-lowered fixture plus the producer facts the test consumes.
struct SourceLoweredFixture {
    lowered: LoweredFixture,
    discharged: Vec<(HirNodeId, HirNumericOp)>,
    numeric_invokes: usize,
}

/// Lowers one function from real Moth source and then feeds the same HIR to the JS backend.
///
/// WHY: mandatory discharge is a producer contract. A hand-built statement can only reproduce
///      its shape; lowering the source proves the frontend actually selects `Infallible` and that
///      the backend consumes it without any proof-table fact.
fn lower_source_fixture(source: &str) -> SourceLoweredFixture {
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let registry = default_external_package_registry(&mut string_table);
    let lowering = lower_hir_result(ast, &mut string_table, &mut path_fork, &registry);
    let module = lowering.hir_module;
    let type_environment = lowering.type_environment;
    validate_hir_module(&module, &type_environment, &registry)
        .expect("source-lowered numeric fixture must satisfy production HIR validation");
    let function_id = module
        .functions
        .iter()
        .map(|function| function.id)
        .find(|id| Some(*id) != module.start_function)
        .expect("the source module must contain the fixture function beside its entry start");
    let discharged = module
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match &statement.kind {
            HirStatementKind::NumericOp {
                op,
                failure_mode: NumericFailureMode::Infallible,
                ..
            } => Some((statement.id, *op)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let numeric_invokes = module
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                &block.terminator,
                HirTerminator::Invoke {
                    operation: HirFallibleOperation::NumericOp { .. },
                    ..
                }
            )
        })
        .count();

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
    let function_name = analysed_module
        .function_name_by_id
        .get(&function_id)
        .cloned()
        .expect("the source function is emitted");

    SourceLoweredFixture {
        lowered: LoweredFixture {
            analysed_source: analysed_module.source,
            retained_source: retained_module.source,
            analysed_table,
            function_name,
        },
        discharged,
        numeric_invokes,
    }
}

/// Verifies the mandatory `Infallible` mode survives from source lowering to scalar JS.
///
/// WHAT: lowers a real mixed-width `I8 + U8 -> I16` addition, asserts HIR delivers an
///       `Infallible` statement, then lowers under the empty proof table.
/// WHY: the frontend discharge from complete operand type ranges is the producer contract that
///       must lower to a scalar without any `NumericProofs` entry. The empty table therefore
///       proves the statement mode alone removed the checked helper, trap machinery and
///       `{ tag, value }` carrier.
#[test]
fn infallible_statement_mode_lowers_scalar_with_an_empty_proof_table() {
    let source = "\
mixed_add |left I8, right U8| -> I16:
    return left + right
;
";
    let fixture = lower_source_fixture(source);

    assert_eq!(
        fixture.discharged.len(),
        1,
        "the mixed-width add must deliver exactly one Infallible statement"
    );
    let (statement_id, operation) = fixture.discharged[0];
    assert_eq!(
        operation.domain,
        NumericScalar::Fixed(FixedScalar::I16),
        "the mixed-width add completes in its I16 common domain"
    );
    assert_eq!(operation.operator, NumericOperator::Add);
    assert_eq!(
        fixture.numeric_invokes, 0,
        "a discharged statement is not an Invoke"
    );
    assert!(
        fixture
            .lowered
            .analysed_table
            .integer_operation_is_safe(statement_id),
        "the optional analysis proves the same complete operand ranges; the empty-table lowering \
         below is the check that the mandatory mode alone is sufficient"
    );

    for source in [
        &fixture.lowered.analysed_source,
        &fixture.lowered.retained_source,
    ] {
        for helper in [
            "function __moth_int_add(",
            "__moth_int_add(",
            "function __moth_int_check(",
            "__moth_numeric_trap(",
        ] {
            assert!(
                !source.contains(helper),
                "a discharged statement must not reference or emit {helper}"
            );
        }
    }
    let driver = format!("console.log({}(-5, 250));", fixture.lowered.function_name);
    let (analysed, retained) = run_both_tables(&fixture.lowered, &driver);
    assert_same_runtime(&analysed, &retained, "discharged statement");
    assert_eq!(
        analysed.stdout.trim(),
        "245",
        "the discharged add must keep its exact I16 value"
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

/// Lowers one proven numeric Invoke whose exclusive failure block demands its own checked family.
///
/// WHAT: the success block returns the `I32` result; the failure block adds two unknown `I64`
///       parameters - never proved by analysis - demanding `__moth_bigint_add` and the trap
///       before returning the error. `success_slot_in_failure_block` declares the success slot in
///       the pruned failure block instead of the entry block.
/// WHY: the failure block is unreachable once the proof discharges the Invoke, so helper absence
///      proves live-edge pruning skipped it. The unproved add makes helper absence independent of
///      the proof table, and the moved success slot regresses declaration soundness for a local
///      whose declaring block only the pruned edge reaches.
fn lower_exclusive_handler_fixture(
    name: &str,
    success_slot_in_failure_block: bool,
) -> LoweredFixture {
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let mut expressions = HirExpressionStore::default();
    let i64_ty = builtin_type_ids::fixed_scalar(FixedScalar::I64);
    let (result, error, dead, left, right) =
        (LocalId(0), LocalId(1), LocalId(2), LocalId(3), LocalId(4));

    let invoke = HirTerminator::Invoke {
        id: HirNodeId(2),
        span: None,
        operation: HirFallibleOperation::NumericOp {
            op: HirNumericOp::ordinary(
                NumericOperator::Add,
                NumericScalar::Fixed(FixedScalar::I32),
            ),
            operands: binary_operands(
                i32_expression(2, types.i32, region, &mut expressions),
                i32_expression(3, types.i32, region, &mut expressions),
            ),
        },
        success: BlockId(1),
        success_results: vec![result],
        failure: BlockId(2),
        error_result: error,
    };
    let dead_add = statement(
        3,
        trap_op(
            NumericOperator::Add,
            NumericScalar::Fixed(FixedScalar::I64),
            binary_operands(
                load_local(left.0, i64_ty, region, &mut expressions),
                load_local(right.0, i64_ty, region, &mut expressions),
            ),
            dead.0,
        ),
    );
    let success_return = load_local(result.0, types.i32, region, &mut expressions);
    let failure_error = load_local(error.0, types.error, region, &mut expressions);

    let mut entry_locals = vec![
        local(error.0, types.error, region),
        local(left.0, i64_ty, region),
        local(right.0, i64_ty, region),
    ];
    let mut failure_locals = vec![local(dead.0, i64_ty, region)];
    if success_slot_in_failure_block {
        failure_locals.push(local(result.0, types.i32, region));
    } else {
        entry_locals.push(local(result.0, types.i32, region));
    }

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region,
            locals: entry_locals,
            statements: vec![],
            terminator: invoke,
        },
        HirBlock {
            id: BlockId(1),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(append_values(&[success_return], &mut expressions)),
        },
        HirBlock {
            id: BlockId(2),
            region,
            locals: failure_locals,
            statements: vec![dead_add],
            terminator: HirTerminator::ReturnError(failure_error),
        },
    ];
    lower_numeric_blocks(
        name,
        expressions,
        type_environment,
        blocks,
        HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![left, right],
            return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
                vec![types.i32],
                Some(types.error),
            )),
        },
        vec![
            (result, "result"),
            (error, "error"),
            (dead, "dead"),
            (left, "left"),
            (right, "right"),
        ],
    )
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
                kind: RuntimeFailureKind::NoMatchFallback,
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
            return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
                returns,
                Some(types.error),
            )),
        },
        names,
    )
}

#[test]
fn proven_invoke_elides_checks_and_preserves_success_abi() {
    let lowered = lower_exclusive_handler_fixture("proven_invoke", false);
    assert!(
        lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(2))
    );
    assert!(
        !lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(3)),
        "the handler's parametric I64 add must stay unproved so helper absence tests pruning"
    );
    assert!(!lowered.analysed_source.contains("function __moth_int_add("));
    assert!(!lowered.analysed_source.contains("__moth_int_check("));
    assert!(!lowered.analysed_source.contains("__moth_numeric_trap("));
    assert!(
        !lowered
            .analysed_source
            .contains("function __moth_bigint_add("),
        "the exclusive handler's checked family must not be demanded"
    );
    assert!(
        !lowered.analysed_source.contains("if (true)"),
        "a proven numeric Invoke must not emit a dead success conditional"
    );
    assert!(
        !lowered.analysed_source.contains(".tag === \"ok\""),
        "a proven numeric Invoke must not test its scalar result as a carrier"
    );
    assert!(
        !lowered
            .analysed_source
            .contains("tag: \"err\", value: __moth_read("),
        "the proven Invoke's exclusive failure handler must not be emitted"
    );
    assert!(
        lowered.retained_source.contains(".tag === \"ok\""),
        "the empty proof table keeps the checked success branch"
    );
    assert!(
        lowered
            .retained_source
            .contains("function __moth_bigint_add("),
        "the empty proof table reaches the handler and demands its checked family"
    );
    let driver = format!(
        "console.log(JSON.stringify({}(1n, 2n)));",
        lowered.function_name
    );
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(&analysed, &retained, "proven Invoke success ABI");
    assert_eq!(analysed.stdout.trim(), r#"{"tag":"ok","value":5}"#);
}

/// A chained Invoke proves its second operation only because the first proof seeds the sole
/// success block's entry interval.
///
/// WHAT: lowers `wide(left) + wide(right)`, then `sum + 7`, then a real division by the chained
///       result under the analysed and empty tables and runs three inputs in Node.
/// WHY: the second add is inside the first Invoke's sole success block. Without the seed its
///      canonical I32 operand bounds admit overflow, so the checked add and its helper would
///      remain. The division stays checked and really fails when the chain reaches zero, so both
///      success and error results stay observable across tables.
#[test]
fn seeded_success_block_proves_the_next_invoke_and_preserves_runtime_parity() {
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let mut expressions = HirExpressionStore::default();
    let i16_ty = builtin_type_ids::fixed_scalar(FixedScalar::I16);
    let (sum, biased, quotient, error, left, right) = (
        LocalId(0),
        LocalId(1),
        LocalId(2),
        LocalId(3),
        LocalId(4),
        LocalId(5),
    );

    let cast_to_i32 = |source, expressions: &mut HirExpressionStore| {
        expression(
            HirExpressionKind::Cast {
                source,
                policy: BuiltinCastPolicyId::NumericConversion {
                    source: NumericScalar::Fixed(FixedScalar::I16),
                    target: NumericScalar::Fixed(FixedScalar::I32),
                },
            },
            types.i32,
            region,
            ValueKind::RValue,
            expressions,
        )
    };
    let left_value = load_local(left.0, i16_ty, region, &mut expressions);
    let wide_left = cast_to_i32(left_value, &mut expressions);
    let right_value = load_local(right.0, i16_ty, region, &mut expressions);
    let wide_right = cast_to_i32(right_value, &mut expressions);
    let first_invoke = HirTerminator::Invoke {
        id: HirNodeId(0),
        span: None,
        operation: HirFallibleOperation::NumericOp {
            op: HirNumericOp::ordinary(
                NumericOperator::Add,
                NumericScalar::Fixed(FixedScalar::I32),
            ),
            operands: binary_operands(wide_left, wide_right),
        },
        success: BlockId(1),
        success_results: vec![sum],
        failure: BlockId(4),
        error_result: error,
    };

    let sum_load = load_local(sum.0, types.i32, region, &mut expressions);
    let seven = i32_expression(7, types.i32, region, &mut expressions);
    let second_invoke = HirTerminator::Invoke {
        id: HirNodeId(1),
        span: None,
        operation: HirFallibleOperation::NumericOp {
            op: HirNumericOp::ordinary(
                NumericOperator::Add,
                NumericScalar::Fixed(FixedScalar::I32),
            ),
            operands: binary_operands(sum_load, seven),
        },
        success: BlockId(2),
        success_results: vec![biased],
        failure: BlockId(4),
        error_result: error,
    };

    let million = i32_expression(1_000_000, types.i32, region, &mut expressions);
    let biased_load = load_local(biased.0, types.i32, region, &mut expressions);
    let third_invoke = HirTerminator::Invoke {
        id: HirNodeId(2),
        span: None,
        operation: HirFallibleOperation::NumericOp {
            op: HirNumericOp::ordinary(
                NumericOperator::IntegerDivide,
                NumericScalar::Fixed(FixedScalar::I32),
            ),
            operands: binary_operands(million, biased_load),
        },
        success: BlockId(3),
        success_results: vec![quotient],
        failure: BlockId(4),
        error_result: error,
    };

    let success_value = load_local(quotient.0, types.i32, region, &mut expressions);
    let success_return = append_values(&[success_value], &mut expressions);
    let failure_error = load_local(error.0, types.error, region, &mut expressions);

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region,
            locals: vec![
                local(sum.0, types.i32, region),
                local(biased.0, types.i32, region),
                local(quotient.0, types.i32, region),
                local(error.0, types.error, region),
                local(left.0, i16_ty, region),
                local(right.0, i16_ty, region),
            ],
            statements: vec![],
            terminator: first_invoke,
        },
        HirBlock {
            id: BlockId(1),
            region,
            locals: vec![],
            statements: vec![],
            terminator: second_invoke,
        },
        HirBlock {
            id: BlockId(2),
            region,
            locals: vec![],
            statements: vec![],
            terminator: third_invoke,
        },
        HirBlock {
            id: BlockId(3),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(success_return),
        },
        HirBlock {
            id: BlockId(4),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::ReturnError(failure_error),
        },
    ];

    let lowered = lower_numeric_blocks(
        "seeded_invoke_chain",
        expressions,
        type_environment,
        blocks,
        HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![left, right],
            return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
                vec![types.i32],
                Some(types.error),
            )),
        },
        vec![
            (sum, "sum"),
            (biased, "biased"),
            (quotient, "quotient"),
            (error, "error"),
            (left, "left"),
            (right, "right"),
        ],
    );

    assert!(
        lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(0))
    );
    assert!(
        lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(1)),
        "the sole success block's seeded interval must prove the chained add"
    );
    assert!(
        !lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(2)),
        "the division may divide by the chained zero result and must stay checked"
    );
    assert_eq!(
        lowered.analysed_source.matches("= __moth_int_add(").count(),
        0,
        "the first and chained adds must both lower to scalars without checked helper calls"
    );
    assert_eq!(
        lowered.retained_source.matches("= __moth_int_add(").count(),
        2,
        "the empty proof table must keep both checked add calls"
    );
    assert!(
        lowered.analysed_source.contains("function __moth_int_div("),
        "the checked division must keep its helper"
    );

    let driver = format!(
        "const show = (result) => result.tag === \"ok\" ? String(result.value) : \"err:\" + __moth_error_code(result.value);\n\
         console.log(show({}(200, 300)));\n\
         console.log(show({}(0, -7)));\n\
         console.log(show({}(0, -8)));",
        lowered.function_name, lowered.function_name, lowered.function_name
    );
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(&analysed, &retained, "seeded Invoke chain");
    assert!(
        analysed.success,
        "the seeded chain must execute: {}",
        analysed.stderr
    );
    assert_eq!(analysed.stdout, "1972\nerr:300\n-1000000\n");
}

/// A success slot declared only in a pruned failure block still reserves its JS name.
///
/// WHAT: lowers the proven Invoke with `success_results` declared by the failure block, then
///       executes the analysed module in Node.
/// WHY: HIR validation only checks same-function ownership, not declaration dominance, so the
///      live success transfer must not write an undeclared name when the declaring block is
///      reachable only through the pruned failure edge.
#[test]
fn success_slot_declared_in_a_pruned_block_still_gets_a_declaration() {
    let lowered = lower_exclusive_handler_fixture("pruned_declaration", true);
    let result_name = expected_dev_local_name("result", 0);
    assert!(
        lowered
            .analysed_source
            .contains(&format!("let {result_name};")),
        "a success slot declared in the pruned failure block must still reserve its name"
    );
    assert!(
        !lowered.analysed_source.contains("if (true)"),
        "the failure edge must still be pruned for this regression"
    );
    let driver = format!(
        "console.log(JSON.stringify({}(1n, 2n)));",
        lowered.function_name
    );
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert!(
        analysed.success,
        "the analysed module must execute without an undeclared-name error: {}",
        analysed.stderr
    );
    assert_same_runtime(&analysed, &retained, "pruned-block success slot");
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
                        op: HirNumericOp::ordinary(
                            NumericOperator::Add,
                            NumericScalar::Fixed(FixedScalar::I32),
                        ),
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

/// Lowers a four-Invoke fixture whose checked siblings share the proven Invokes' handler.
///
/// WHAT: block 0 invokes a literal-proven I32 add; block 1 invokes a literal-proven I32-to-I8
///       narrowing; block 2 invokes a checked I32 add whose operand comes from the function
///       parameter; block 3 invokes a checked I32-to-I8 narrowing of the same policy on the
///       checked add's result. The proven and checked numeric adds share one handler, and the
///       proven and checked narrowings share a second cast handler. `cyclic` inserts a
///       literal-false back edge, which forces the dispatcher instead of structured emission.
/// WHY: the proven Invokes' failure edges are dead while each shared handler is still live through
///       its checked sibling, so both handlers must stay emitted and recover in either proof table
///       and under both control-flow strategies. The runtime calls reach the checked add failure,
///       the checked narrowing failure, and the all-safe path, proving the shared cast handler
///       still serves the checked narrow while the proven narrow needs no failure path.
fn lower_shared_handler_fixture(name: &str, cyclic: bool) -> LoweredFixture {
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let i8_ty = builtin_type_ids::fixed_scalar(FixedScalar::I8);
    let mut expressions = HirExpressionStore::default();
    let first_result = LocalId(0);
    // One error local for every failure edge: the handler is reachable from each of them, so a
    // definition must dominate its use on all incoming paths.
    let error = LocalId(1);
    let second_result = LocalId(2);
    let proven_cast_result = LocalId(3);
    let checked_cast_result = LocalId(4);
    let input = LocalId(5);
    // A dense block table keeps the fixture identical to production compacted HIR.
    let (checked_cast_success, return_block, cast_handler, add_handler) = if cyclic {
        (BlockId(4), BlockId(5), BlockId(6), BlockId(7))
    } else {
        (BlockId(4), BlockId(4), BlockId(5), BlockId(6))
    };

    let proven_add = HirTerminator::Invoke {
        id: HirNodeId(2),
        span: None,
        operation: HirFallibleOperation::NumericOp {
            op: HirNumericOp::ordinary(
                NumericOperator::Add,
                NumericScalar::Fixed(FixedScalar::I32),
            ),
            operands: binary_operands(
                i32_expression(2, types.i32, region, &mut expressions),
                i32_expression(3, types.i32, region, &mut expressions),
            ),
        },
        success: BlockId(1),
        success_results: vec![first_result],
        failure: add_handler,
        error_result: error,
    };
    let proven_cast = HirTerminator::Invoke {
        id: HirNodeId(6),
        span: None,
        operation: narrowing_cast(
            FixedScalar::I32,
            FixedScalar::I8,
            i32_expression(100, types.i32, region, &mut expressions),
        ),
        success: BlockId(2),
        success_results: vec![proven_cast_result],
        failure: cast_handler,
        error_result: error,
    };
    let checked_add = HirTerminator::Invoke {
        id: HirNodeId(4),
        span: None,
        operation: HirFallibleOperation::NumericOp {
            op: HirNumericOp::ordinary(
                NumericOperator::Add,
                NumericScalar::Fixed(FixedScalar::I32),
            ),
            operands: binary_operands(
                load_local(input.0, types.i32, region, &mut expressions),
                i32_expression(3, types.i32, region, &mut expressions),
            ),
        },
        success: BlockId(3),
        success_results: vec![second_result],
        failure: add_handler,
        error_result: error,
    };
    let checked_cast = HirTerminator::Invoke {
        id: HirNodeId(8),
        span: None,
        operation: narrowing_cast(
            FixedScalar::I32,
            FixedScalar::I8,
            load_local(second_result.0, types.i32, region, &mut expressions),
        ),
        success: checked_cast_success,
        success_results: vec![checked_cast_result],
        failure: cast_handler,
        error_result: error,
    };
    let success_return = HirTerminator::Return(append_values(
        &[load_local(
            second_result.0,
            types.i32,
            region,
            &mut expressions,
        )],
        &mut expressions,
    ));
    let cyclic_terminator = HirTerminator::If {
        condition: bool_expression(false, types.boolean, region, &mut expressions),
        then_block: BlockId(3),
        else_block: return_block,
    };

    let mut blocks = vec![
        HirBlock {
            id: BlockId(0),
            region,
            locals: vec![
                local(first_result.0, types.i32, region),
                local(error.0, types.error, region),
                local(second_result.0, types.i32, region),
                local(proven_cast_result.0, i8_ty, region),
                local(checked_cast_result.0, i8_ty, region),
                local(input.0, types.i32, region),
            ],
            statements: vec![],
            terminator: proven_add,
        },
        HirBlock {
            id: BlockId(1),
            region,
            locals: vec![],
            statements: vec![],
            terminator: proven_cast,
        },
        HirBlock {
            id: BlockId(2),
            region,
            locals: vec![],
            statements: vec![],
            terminator: checked_add,
        },
        HirBlock {
            id: BlockId(3),
            region,
            locals: vec![],
            statements: vec![],
            terminator: checked_cast,
        },
        HirBlock {
            id: BlockId(4),
            region,
            locals: vec![],
            statements: vec![],
            terminator: if cyclic {
                cyclic_terminator
            } else {
                success_return.clone()
            },
        },
    ];
    if cyclic {
        blocks.push(HirBlock {
            id: return_block,
            region,
            locals: vec![],
            statements: vec![],
            terminator: success_return,
        });
    }
    blocks.push(HirBlock {
        id: cast_handler,
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::ReturnError(load_local(
            error.0,
            types.error,
            region,
            &mut expressions,
        )),
    });
    blocks.push(HirBlock {
        id: add_handler,
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::ReturnError(load_local(
            error.0,
            types.error,
            region,
            &mut expressions,
        )),
    });

    lower_numeric_blocks(
        name,
        expressions,
        type_environment,
        blocks,
        HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![input],
            return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
                vec![types.i32],
                Some(types.error),
            )),
        },
        vec![
            (first_result, "first"),
            (error, "error"),
            (second_result, "second"),
            (proven_cast_result, "proven_cast"),
            (checked_cast_result, "checked_cast"),
            (input, "input"),
        ],
    )
}

/// Verifies a scalar numeric Invoke inside one structured `If` arm reaches the shared join.
///
/// WHAT: the then arm performs a literal-proven I32 add whose scalar Invoke stores its result;
///       that success continuation publishes the result into a pre-declared local and jumps to
///       the join. The else arm jumps to the same join. The join copies the local once and returns
///       it.
/// WHY: structured lowering must emit that continuation once after the `if`/`else` rather than
///      duplicating it inside the scalar Invoke's success continuation. Running both arms proves
///      each path crosses the single shared return with its own value.
#[test]
fn scalar_invoke_inside_structured_if_arm_reaches_the_shared_join_once() {
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let mut expressions = HirExpressionStore::default();
    let (result, error, base, flag, joined) =
        (LocalId(0), LocalId(1), LocalId(2), LocalId(3), LocalId(4));

    let invoke = HirTerminator::Invoke {
        id: HirNodeId(1),
        span: None,
        operation: HirFallibleOperation::NumericOp {
            op: HirNumericOp::ordinary(
                NumericOperator::Add,
                NumericScalar::Fixed(FixedScalar::I32),
            ),
            operands: binary_operands(
                i32_expression(2, types.i32, region, &mut expressions),
                i32_expression(3, types.i32, region, &mut expressions),
            ),
        },
        success: BlockId(3),
        success_results: vec![result],
        failure: BlockId(5),
        error_result: error,
    };
    let seed_base = statement(
        4,
        define_local(
            base.0,
            i32_expression(10, types.i32, region, &mut expressions),
        ),
    );
    let result_load = load_local(result.0, types.i32, region, &mut expressions);
    let publish = statement(
        3,
        HirStatementKind::Write {
            target: HirWriteTarget::AssignPlace(HirPlace::local(base)),
            value: result_load,
        },
    );
    let join_write = statement(
        2,
        define_local(
            joined.0,
            load_local(base.0, types.i32, region, &mut expressions),
        ),
    );
    let joined_return = load_local(joined.0, types.i32, region, &mut expressions);
    let error_load = load_local(error.0, types.error, region, &mut expressions);

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region,
            locals: vec![
                local(result.0, types.i32, region),
                local(error.0, types.error, region),
                local(base.0, types.i32, region),
                local(flag.0, types.boolean, region),
                local(joined.0, types.i32, region),
            ],
            statements: vec![seed_base],
            terminator: HirTerminator::If {
                condition: load_local(flag.0, types.boolean, region, &mut expressions),
                then_block: BlockId(1),
                else_block: BlockId(2),
            },
        },
        HirBlock {
            id: BlockId(1),
            region,
            locals: vec![],
            statements: vec![],
            terminator: invoke,
        },
        HirBlock {
            id: BlockId(2),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Jump {
                target: BlockId(4),
                args: vec![],
            },
        },
        // The scalar Invoke's success continuation publishes its result, then reaches the join.
        HirBlock {
            id: BlockId(3),
            region,
            locals: vec![],
            statements: vec![publish],
            terminator: HirTerminator::Jump {
                target: BlockId(4),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(4),
            region,
            locals: vec![],
            statements: vec![join_write],
            terminator: HirTerminator::Return(append_values(&[joined_return], &mut expressions)),
        },
        HirBlock {
            id: BlockId(5),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::ReturnError(error_load),
        },
    ];

    let lowered = lower_numeric_blocks(
        "structured_join",
        expressions,
        type_environment,
        blocks,
        HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![flag],
            return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
                vec![types.i32],
                Some(types.error),
            )),
        },
        vec![
            (result, "result"),
            (error, "error"),
            (base, "base"),
            (joined, "joined"),
            (flag, "flag"),
        ],
    );

    assert!(
        lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(1)),
        "the literal add inside the then arm must be proven and lowered as a scalar Invoke"
    );
    for source in [&lowered.analysed_source, &lowered.retained_source] {
        assert!(
            !source.contains("switch (__bb"),
            "an acyclic If with one scalar-Invoke arm must stay on structured lowering"
        );
        assert!(
            source.contains("if ("),
            "the fixture must emit a structured conditional"
        );
    }
    assert!(
        !lowered.analysed_source.contains("if (true)"),
        "a proven scalar Invoke must not emit a dead success conditional"
    );
    let joined_name = expected_dev_local_name("joined", 4);
    assert_eq!(
        lowered
            .analysed_source
            .matches(&format!("{joined_name} = "))
            .count(),
        1,
        "the shared join must be emitted once, after the if/else"
    );

    let driver = format!(
        "const yes = {}(true);\nconst no = {}(false);\nconsole.log(yes.tag, yes.value, no.tag, no.value);",
        lowered.function_name, lowered.function_name
    );
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(&analysed, &retained, "structured If scalar Invoke join");
    assert_eq!(
        analysed.stdout.trim(),
        "ok 5 ok 10",
        "each arm must reach the shared continuation once with its own value"
    );
}

#[test]
fn scalar_invoke_dead_failure_edge_keeps_a_shared_live_handler() {
    for cyclic in [false, true] {
        let lowered = lower_shared_handler_fixture("shared_handler", cyclic);
        let strategy = if cyclic { "dispatcher" } else { "structured" };
        assert!(
            lowered
                .analysed_table
                .integer_operation_is_safe(HirNodeId(2)),
            "{strategy}: the literal add must be proven"
        );
        assert!(
            lowered
                .analysed_table
                .integer_narrowing_is_safe(HirNodeId(6)),
            "{strategy}: the literal narrowing must be proven"
        );
        assert!(
            !lowered
                .analysed_table
                .integer_operation_is_safe(HirNodeId(4)),
            "{strategy}: the parameter-fed add must stay checked"
        );
        assert!(
            !lowered
                .analysed_table
                .integer_narrowing_is_safe(HirNodeId(8)),
            "{strategy}: the narrowing of the checked add result must stay checked"
        );
        assert_eq!(
            lowered.analysed_source.contains("switch (__bb"),
            cyclic,
            "{strategy}: the literal-false back edge must select the expected control-flow strategy"
        );
        assert!(
            !lowered.analysed_source.contains("if (true)"),
            "{strategy}: a proven Invoke must not emit a dead success conditional"
        );
        assert_eq!(
            lowered.analysed_source.matches(".tag === \"ok\"").count(),
            2,
            "{strategy}: only the checked add and checked narrowing keep carrier branches"
        );
        assert_eq!(
            lowered.retained_source.matches(".tag === \"ok\"").count(),
            4,
            "{strategy}: the empty proof table keeps all four checked branches"
        );
        assert!(
            lowered
                .analysed_source
                .contains("function __moth_cast_integer_to_integer("),
            "{strategy}: the checked narrowing keeps its cast helper"
        );
        assert!(
            lowered
                .analysed_source
                .contains("tag: \"err\", value: __moth_read("),
            "{strategy}: the shared cast handler stays live through the checked narrowing"
        );
        // The shared error slot is the handler's incoming binding. Only the two checked siblings
        // may clear or write it once the proven Invokes' failure edges are pruned.
        let error_name = expected_dev_local_name("error", 1);
        let clear = format!("{error_name} = undefined;");
        let write = format!("{error_name} = __moth_binding(");
        for (source, expected, table) in [
            (&lowered.analysed_source, 2, "analysed"),
            (&lowered.retained_source, 4, "retained"),
        ] {
            assert_eq!(
                source.matches(&clear).count(),
                expected,
                "{strategy} {table}: error slot clears"
            );
            assert_eq!(
                source.matches(&write).count(),
                expected,
                "{strategy} {table}: error slot writes"
            );
        }

        let driver = format!(
            "const castFailure = {f}(300);\n\
             const addFailure = {f}({max});\n\
             const success = {f}(0);\n\
             console.log(castFailure.tag + \":\" + __moth_error_code(castFailure.value));\n\
             console.log(addFailure.tag + \":\" + __moth_error_code(addFailure.value));\n\
             console.log(success.tag + \" \" + success.value);",
            f = lowered.function_name,
            max = i32::MAX
        );
        let (analysed, retained) = run_both_tables(&lowered, &driver);
        assert_same_runtime(&analysed, &retained, strategy);
        assert_eq!(
            analysed.stdout.trim(),
            format!(
                "err:{cast}\nerr:{add}\nok 3",
                cast = BuiltinErrorCode::IntCastOutOfRange.as_u32(),
                add = BuiltinErrorCode::IntOverflow.as_u32()
            ),
            "{strategy}: the shared handler must recover the checked narrowing while the numeric sibling recovers its overflow and the proven narrowing stays out of the failure path"
        );
    }
}

/// Narrowing elides only its predicate, preserving physical conversion and the fallible ABI.
#[test]
fn proven_narrowing_elides_checked_cast_helpers() {
    for (source, target, value) in [
        (FixedScalar::I32, FixedScalar::I8, 100),
        (FixedScalar::I64, FixedScalar::I32, 1000),
        (FixedScalar::I32, FixedScalar::U64, 100),
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
        assert!(
            !lowered.analysed_source.contains("if (true)"),
            "a proven narrowing must not emit a dead success conditional"
        );
        assert_eq!(
            lowered.analysed_source.matches(".tag === \"ok\"").count(),
            0,
            "a proven narrowing transfers success unconditionally"
        );
        assert_eq!(
            lowered.retained_source.matches(".tag === \"ok\"").count(),
            1,
            "the empty proof table keeps the checked narrowing branch"
        );
        let driver = format!(
            "const result = {}(); console.log(result.tag, String(result.value), typeof result.value);",
            lowered.function_name
        );
        let (analysed, retained) = run_both_tables(&lowered, &driver);
        assert_same_runtime(&analysed, &retained, "proven narrowing ABI");
        assert_eq!(
            analysed.stdout.trim(),
            format!(
                "ok {value} {}",
                if target == FixedScalar::U64 {
                    "bigint"
                } else {
                    "number"
                }
            )
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
    assert!(
        !lowered.analysed_source.contains("if (true)"),
        "the proven sibling must not leave a dead success conditional"
    );
    assert_eq!(
        lowered.analysed_source.matches(".tag === \"ok\"").count(),
        1,
        "only the checked sibling keeps its carrier branch"
    );
    assert_eq!(
        lowered.retained_source.matches(".tag === \"ok\"").count(),
        2,
        "the empty proof table keeps both checked narrowing branches"
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
