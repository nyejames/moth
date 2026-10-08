//! Executable parity tests for the numeric proof table consumer.
//!
//! WHAT: lowers the same hand-built HIR twice — once with the analysed proof table and once with
//!       the empty default table that retains every check — and executes both generated modules
//!       in Node.js.
//! WHY: proven integer operations and narrowings must produce equal exact results, error codes
//!       and effect ordering while omitting only the checked machinery the proof makes
//!       unreachable, including signed-zero normalization and required carrier shapes.

use super::support::*;
use crate::compiler_frontend::analysis::numeric_proofs::analyse_numeric_proofs;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::datatypes::definitions::StructTypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId};
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
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
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::validate_hir_module;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use std::process::Command;

/// The builtin `Error` type id, registered through the same canonical-identity dance the
/// analysis invariants fixtures use, because `TypeEnvironment::new()` does not seed it.
fn fixture_error_type(type_environment: &mut TypeEnvironment) -> TypeId {
    let error_identity = CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error);
    if let Some(error_type_id) = type_environment.type_id_for_canonical_identity(&error_identity) {
        return error_type_id;
    }

    let (_, error_type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: PathId::ROOT,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    type_environment
        .register_canonical_identity(error_identity, error_type_id)
        .expect("test builtin Error identity should register");
    error_type_id
}

/// Interned internal fallible carrier carrying the builtin `Error` as its error payload,
/// exactly like the carriers validated HIR gives ReturnError and CastOp result locals.
fn fixture_carrier(type_environment: &mut TypeEnvironment, success: TypeId) -> TypeId {
    let error_type = fixture_error_type(type_environment);
    type_environment.intern_fallible_carrier(success, error_type)
}

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
    profile: NumericProfile,
    build: impl FnOnce(
        &TypeIds,
        &mut TypeEnvironment,
        RegionId,
        &mut HirExpressionStore,
    ) -> NumericFixture,
) -> LoweredFixture {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (mut type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let mut expressions = HirExpressionStore::default();
    let fixture = build(&types, &mut type_environment, region, &mut expressions);
    let return_type = expressions.expression(fixture.return_value).ty;

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: fixture.locals,
        statements: fixture.statements,
        terminator: HirTerminator::Return(fixture.return_value),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type,
    };
    // Hand-built fixtures carry the same entry-start function-origin tag production lowering
    // records, so the fixture shape matches what HIR validation accepts.
    let mut module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        function_name,
        vec![block],
        function,
        &fixture.local_names,
    );
    module
        .function_origins
        .insert(FunctionId(0), HirFunctionOrigin::EntryStart);
    validate_hir_module(&module, &type_environment)
        .expect("numeric fixture HIR must satisfy production HIR validation");

    let analysed_table = analyse_numeric_proofs(&module, &type_environment, profile);
    let config = JsLoweringConfig::direct_js(false, profile);
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

fn assign_local(
    target: u32,
    value: crate::compiler_frontend::hir::ids::HirValueId,
) -> HirStatementKind {
    HirStatementKind::Write {
        target: HirWriteTarget::AssignPlace(HirPlace::local(LocalId(target))),
        value,
    }
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

fn return_error_op(
    operator: NumericOperator,
    domain: NumericScalar,
    left: crate::compiler_frontend::hir::ids::HirValueId,
    right: crate::compiler_frontend::hir::ids::HirValueId,
    result: u32,
) -> HirStatementKind {
    HirStatementKind::NumericOp {
        op: HirNumericOp { operator, domain },
        failure_mode: NumericFailureMode::ReturnError,
        operands: binary_operands(left, right),
        result: HirLocalDestination::Define(LocalId(result)),
    }
}

fn narrowing_cast(
    source_scalar: FixedScalar,
    target_scalar: FixedScalar,
    source: crate::compiler_frontend::hir::ids::HirValueId,
    result: u32,
) -> HirStatementKind {
    HirStatementKind::CastOp {
        policy: BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(source_scalar),
            target: NumericScalar::Fixed(target_scalar),
        },
        source,
        result: Some(HirLocalDestination::Define(LocalId(result))),
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
        NumericProfile::STANDARD,
        |types, _environment, region, expressions| {
            let (locals, local_names) = named_locals(
                &[
                    (0, types.int, "a"),
                    (1, types.int, "b"),
                    (2, types.int, "c"),
                    (3, types.int, "d"),
                    (4, types.int, "e"),
                    (5, types.int, "f"),
                ],
                region,
            );
            let int = types.int;
            NumericFixture {
                locals,
                local_names,
                statements: vec![
                    statement(
                        1,
                        define_local(0, int_expression(2, int, region, expressions)),
                    ),
                    statement(
                        2,
                        trap_op(
                            NumericOperator::Add,
                            NumericScalar::Int,
                            binary_operands(
                                load_local(0, int, region, expressions),
                                int_expression(3, int, region, expressions),
                            ),
                            1,
                        ),
                    ),
                    statement(
                        3,
                        trap_op(
                            NumericOperator::Multiply,
                            NumericScalar::Int,
                            binary_operands(
                                load_local(1, int, region, expressions),
                                int_expression(4, int, region, expressions),
                            ),
                            2,
                        ),
                    ),
                    statement(
                        4,
                        trap_op(
                            NumericOperator::Subtract,
                            NumericScalar::Int,
                            binary_operands(
                                load_local(2, int, region, expressions),
                                int_expression(5, int, region, expressions),
                            ),
                            3,
                        ),
                    ),
                    statement(
                        5,
                        trap_op(
                            NumericOperator::IntegerDivide,
                            NumericScalar::Int,
                            binary_operands(
                                load_local(3, int, region, expressions),
                                int_expression(2, int, region, expressions),
                            ),
                            4,
                        ),
                    ),
                    statement(
                        6,
                        trap_op(
                            NumericOperator::Remainder,
                            NumericScalar::Int,
                            binary_operands(
                                load_local(4, int, region, expressions),
                                int_expression(4, int, region, expressions),
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
                .integer_operation_is_safe(HirNodeId(statement_id), NumericProfile::STANDARD),
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
    let lowered = lower_numeric_fixture(
        name,
        NumericProfile::STANDARD,
        |types, environment, region, expressions| {
            let (operation, destination) = build_operation(types, environment, region, expressions);
            let (locals, local_names) = named_locals(&[(0, destination, "v")], region);
            NumericFixture {
                locals,
                local_names,
                statements: vec![statement(1, operation)],
                return_value: load_local(0, destination, region, expressions),
            }
        },
    );

    assert!(
        lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(1), NumericProfile::STANDARD),
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
                NumericScalar::Int,
                binary_operands(
                    int_expression(0, types.int, region, expressions),
                    int_expression(-5, types.int, region, expressions),
                ),
                0,
            ),
            types.int,
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
                    domain: NumericScalar::Int,
                },
                failure_mode: NumericFailureMode::Trap,
                operands: HirNumericOperands::Unary {
                    operand: int_expression(0, types.int, region, expressions),
                },
                result: HirLocalDestination::Define(LocalId(0)),
            },
            types.int,
        )
    });
    assert_proven_zero_normalization("zero_div", |types, _environment, region, expressions| {
        (
            trap_op(
                NumericOperator::IntegerDivide,
                NumericScalar::Int,
                binary_operands(
                    int_expression(0, types.int, region, expressions),
                    int_expression(-3, types.int, region, expressions),
                ),
                0,
            ),
            types.int,
        )
    });
    assert_proven_zero_normalization("zero_mod", |types, _environment, region, expressions| {
        (
            trap_op(
                NumericOperator::Remainder,
                NumericScalar::Int,
                binary_operands(
                    int_expression(-7, types.int, region, expressions),
                    int_expression(7, types.int, region, expressions),
                ),
                0,
            ),
            types.int,
        )
    });
}

/// Verifies that a proven standard Int negation of the negative literal `-5` executes in both
/// proof tables and returns the exact positive literal `5` (a fused `--` literal must fail).
#[test]
fn proven_negative_literal_negation_matches_positive_number_literal() {
    let lowered = lower_numeric_fixture(
        "negative_literal_negation_number",
        NumericProfile::STANDARD,
        |types, _environment, region, expressions| {
            let int = types.int;
            let (locals, local_names) = named_locals(&[(0, int, "v")], region);
            NumericFixture {
                locals,
                local_names,
                statements: vec![statement(
                    1,
                    HirStatementKind::NumericOp {
                        op: HirNumericOp {
                            operator: NumericOperator::Negate,
                            domain: NumericScalar::Int,
                        },
                        failure_mode: NumericFailureMode::Trap,
                        operands: HirNumericOperands::Unary {
                            operand: int_expression(-5, int, region, expressions),
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
            .integer_operation_is_safe(HirNodeId(1), NumericProfile::STANDARD),
        "negating -5 stays inside the standard Int domain and must be proven"
    );

    let driver = format!("console.log({}());", lowered.function_name);
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(&analysed, &retained, "negative literal negation (Number)");
    assert!(
        analysed.success,
        "negating a negative Int literal must execute, not fail to parse: {}",
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
        NumericProfile::STANDARD,
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
            .integer_operation_is_safe(HirNodeId(1), NumericProfile::STANDARD),
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
        NumericProfile::STANDARD,
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
            .integer_operation_is_safe(HirNodeId(1), NumericProfile::STANDARD),
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
        NumericProfile::STANDARD,
        |types, _environment, region, expressions| {
            let (locals, local_names) = named_locals(&[(0, types.int, "v")], region);
            NumericFixture {
                locals,
                local_names,
                statements: vec![statement(
                    1,
                    trap_op(
                        NumericOperator::IntegerDivide,
                        NumericScalar::Int,
                        binary_operands(
                            int_expression(5, types.int, region, expressions),
                            int_expression(0, types.int, region, expressions),
                        ),
                        0,
                    ),
                )],
                return_value: load_local(0, types.int, region, expressions),
            }
        },
    );
    assert!(
        !divide_by_zero
            .analysed_table
            .integer_operation_is_safe(HirNodeId(1), NumericProfile::STANDARD),
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
        NumericProfile::STANDARD,
        |types, _environment, region, expressions| {
            let (locals, local_names) = named_locals(&[(0, types.int, "v")], region);
            NumericFixture {
                locals,
                local_names,
                statements: vec![statement(
                    1,
                    trap_op(
                        NumericOperator::Power,
                        NumericScalar::Int,
                        binary_operands(
                            int_expression(2, types.int, region, expressions),
                            int_expression(3, types.int, region, expressions),
                        ),
                        0,
                    ),
                )],
                return_value: load_local(0, types.int, region, expressions),
            }
        },
    );

    assert!(
        !lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(1), NumericProfile::STANDARD),
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

/// Verifies that a proven ReturnError operation lowers to the native result inside the existing
/// `{tag, value}` success carrier — omitting the checked helper family — while the success block
/// returns through the function's fallible success slot, the executed caller observes the
/// returned carrier's tag plus exact value, and the result and statement effect order stay
/// identical to the retained table on the same HIR.
#[test]
fn proven_return_error_lowers_to_native_carrier_with_unchanged_branch() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (mut type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let int = types.int;
    let carrier = fixture_carrier(&mut type_environment, int);
    let error_type = fixture_error_type(&mut type_environment);

    let (locals, local_names) = named_locals(&[(0, types.int, "seed"), (1, carrier, "r")], region);

    let entry_block = HirBlock {
        id: BlockId(0),
        region,
        locals,
        statements: vec![
            statement(
                1,
                define_local(0, int_expression(2, int, region, &mut expressions)),
            ),
            statement(
                2,
                HirStatementKind::NumericOp {
                    op: HirNumericOp {
                        operator: NumericOperator::Add,
                        domain: NumericScalar::Int,
                    },
                    failure_mode: NumericFailureMode::ReturnError,
                    operands: binary_operands(
                        load_local(0, int, region, &mut expressions),
                        int_expression(3, int, region, &mut expressions),
                    ),
                    result: HirLocalDestination::Define(LocalId(1)),
                },
            ),
        ],
        terminator: HirTerminator::FallibleBranch {
            result: load_local(1, carrier, region, &mut expressions),
            success_block: BlockId(1),
            error_block: BlockId(2),
        },
    };
    let success_block = HirBlock {
        id: BlockId(1),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::ReturnSuccess(expression(
            HirExpressionKind::FallibleUnwrapSuccess {
                result: load_local(1, carrier, region, &mut expressions),
            },
            int,
            region,
            ValueKind::RValue,
            &mut expressions,
        )),
    };
    let error_block = HirBlock {
        id: BlockId(2),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::ReturnError(expression(
            HirExpressionKind::FallibleUnwrapError {
                result: load_local(1, carrier, region, &mut expressions),
            },
            error_type,
            region,
            ValueKind::RValue,
            &mut expressions,
        )),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: carrier,
    };
    let mut module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "proven_return_error",
        vec![entry_block, success_block, error_block],
        function,
        &local_names,
    );
    module
        .function_origins
        .insert(FunctionId(0), HirFunctionOrigin::EntryStart);
    validate_hir_module(&module, &type_environment)
        .expect("hand-built fallible fixture HIR must satisfy production HIR validation");

    let analysed_table =
        analyse_numeric_proofs(&module, &type_environment, NumericProfile::STANDARD);
    let config = JsLoweringConfig::direct_js(false, NumericProfile::STANDARD);
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

    assert!(
        analysed_table.integer_operation_is_safe(HirNodeId(2), NumericProfile::STANDARD),
        "the cached-operand ReturnError operation must be proven"
    );
    assert!(
        !analysed_module.source.contains("function __moth_int_add("),
        "a ReturnError operation proven safe must not emit the checked helper family"
    );
    assert!(
        !analysed_module.source.contains("__moth_int_check("),
        "a proven ReturnError operation must not emit the range check helper"
    );
    assert!(
        !analysed_module.source.contains("__moth_numeric_trap("),
        "a proven ReturnError operation must not emit the trap machinery"
    );

    let function_name = analysed_module
        .function_name_by_id
        .get(&FunctionId(0))
        .cloned()
        .expect("emitted module carries its function name");
    let driver = format!("console.log(JSON.stringify({}()));", function_name);
    let analysed = run_javascript(&format!("{}\n{}", analysed_module.source, driver));
    let retained = run_javascript(&format!("{}\n{}", retained_module.source, driver));
    assert_same_runtime(&analysed, &retained, "proven ReturnError carrier branch");
    assert_eq!(analysed.stdout.trim(), r#"{"tag":"ok","value":5}"#);
}

/// Verifies that a ReturnError sibling the analysis must decline keeps the checked helper family
/// and fails at runtime with the actual overflow code, while the proven sibling on the same HIR
/// returns its exact native-carrier result — both observed from one executed module.
#[test]
fn proven_return_error_with_unsafe_sibling_keeps_checked_family() {
    let lowered = lower_numeric_fixture(
        "return_error_sibling",
        NumericProfile::STANDARD,
        |types, environment, region, expressions| {
            let carrier = fixture_carrier(environment, types.int);
            let tuple_type = environment.intern_tuple(vec![types.int, carrier]);
            let (locals, local_names) = named_locals(
                &[
                    (0, types.int, "seed"),
                    (1, carrier, "proven"),
                    (2, carrier, "unsafe"),
                ],
                region,
            );
            let int = types.int;
            NumericFixture {
                locals,
                local_names,
                statements: vec![
                    statement(
                        1,
                        define_local(0, int_expression(2, int, region, expressions)),
                    ),
                    statement(
                        2,
                        return_error_op(
                            NumericOperator::Add,
                            NumericScalar::Int,
                            load_local(0, int, region, expressions),
                            int_expression(3, int, region, expressions),
                            1,
                        ),
                    ),
                    statement(
                        3,
                        assign_local(0, int_expression(i32::MAX as i64, int, region, expressions)),
                    ),
                    statement(
                        4,
                        return_error_op(
                            NumericOperator::Add,
                            NumericScalar::Int,
                            load_local(0, int, region, expressions),
                            int_expression(3, int, region, expressions),
                            2,
                        ),
                    ),
                ],
                return_value: expression(
                    HirExpressionKind::TupleConstruct {
                        elements: append_values(
                            &[
                                expression(
                                    HirExpressionKind::FallibleUnwrapSuccess {
                                        result: load_local(1, carrier, region, expressions),
                                    },
                                    int,
                                    region,
                                    ValueKind::RValue,
                                    expressions,
                                ),
                                load_local(2, carrier, region, expressions),
                            ],
                            expressions,
                        ),
                    },
                    tuple_type,
                    region,
                    ValueKind::RValue,
                    expressions,
                ),
            }
        },
    );

    assert!(
        lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(2), NumericProfile::STANDARD),
        "the cached-operand ReturnError operation must be proven"
    );
    assert!(
        !lowered
            .analysed_table
            .integer_operation_is_safe(HirNodeId(4), NumericProfile::STANDARD),
        "the overflowing ReturnError sibling must stay checked"
    );
    assert!(
        lowered.analysed_source.contains("function __moth_int_add("),
        "a genuinely unsafe ReturnError sibling must keep the checked helper family"
    );

    let driver = format!(
        "const results = {}();\nconsole.log(results[0], __moth_error_code(results[1].value));",
        lowered.function_name
    );
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(
        &analysed,
        &retained,
        "proven ReturnError with unsafe sibling",
    );
    assert_eq!(
        analysed.stdout.trim(),
        format!("5 {}", BuiltinErrorCode::IntOverflow.as_u32()),
        "the proven sibling must return its exact result while the unsafe sibling fails with the actual overflow code"
    );
}

/// Verifies that a proven I32 -> I8 narrowing omits the checked cast helpers while keeping the
/// required `{tag, value}` success carrier.
#[test]
fn proven_narrowing_elides_checked_cast_helpers() {
    let lowered = lower_numeric_fixture(
        "proven_narrow",
        NumericProfile::STANDARD,
        |_types, environment, region, expressions| {
            // The cast policy owns fixed I32 as its source domain, so the fixture literal carries
            // the actual I32 TypeId rather than the profile Int type.
            let i32_ty = builtin_type_ids::fixed_scalar(FixedScalar::I32);
            let i8_ty = builtin_type_ids::fixed_scalar(FixedScalar::I8);
            let carrier = fixture_carrier(environment, i8_ty);
            let (locals, local_names) = named_locals(&[(0, carrier, "n")], region);
            NumericFixture {
                locals,
                local_names,
                statements: vec![statement(
                    1,
                    narrowing_cast(
                        FixedScalar::I32,
                        FixedScalar::I8,
                        fixed_const(
                            FixedScalarValue::signed(FixedScalar::I32, 100).expect("100 fits I32"),
                            i32_ty,
                            region,
                            expressions,
                        ),
                        0,
                    ),
                )],
                return_value: load_local(0, carrier, region, expressions),
            }
        },
    );

    assert!(
        lowered
            .analysed_table
            .integer_narrowing_is_safe(HirNodeId(1), NumericProfile::STANDARD),
        "a source interval inside the I8 range must be proven"
    );
    assert!(
        !lowered
            .analysed_source
            .contains("function __moth_cast_integer_to_integer("),
        "an exclusively proven narrowing policy must not emit the checked cast helper"
    );
    assert!(
        !lowered
            .analysed_source
            .contains("__moth_cast_integer_in_range"),
        "an exclusively proven narrowing policy must not emit the range predicate helper"
    );
    assert!(
        lowered
            .retained_source
            .contains("function __moth_cast_integer_to_integer("),
        "the empty proof table must keep the checked cast helpers"
    );

    let driver = format!("console.log(JSON.stringify({}()));", lowered.function_name);
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(&analysed, &retained, "proven narrowing carrier");
    assert_eq!(analysed.stdout.trim(), r#"{"tag":"ok","value":100}"#);
}

/// Verifies that a proven BigInteger -> Number narrowing performs the original carrier
/// conversion without the range predicate.
#[test]
fn proven_bigint_narrowing_keeps_carrier_conversion() {
    let lowered = lower_numeric_fixture(
        "bigint_narrow",
        NumericProfile::STANDARD,
        |_types, environment, region, expressions| {
            let i32_ty = builtin_type_ids::fixed_scalar(FixedScalar::I32);
            let i64_ty = builtin_type_ids::fixed_scalar(FixedScalar::I64);
            let carrier = fixture_carrier(environment, i32_ty);
            let (locals, local_names) = named_locals(&[(0, carrier, "n")], region);
            NumericFixture {
                locals,
                local_names,
                statements: vec![statement(
                    1,
                    narrowing_cast(
                        FixedScalar::I64,
                        FixedScalar::I32,
                        fixed_const(
                            FixedScalarValue::signed(FixedScalar::I64, 1000)
                                .expect("1000 fits I64"),
                            i64_ty,
                            region,
                            expressions,
                        ),
                        0,
                    ),
                )],
                return_value: load_local(0, carrier, region, expressions),
            }
        },
    );

    assert!(
        lowered
            .analysed_table
            .integer_narrowing_is_safe(HirNodeId(1), NumericProfile::STANDARD),
        "an in-range I64 source must be proven against the I32 target"
    );
    assert!(
        !lowered
            .analysed_source
            .contains("function __moth_cast_integer_to_integer("),
        "an exclusively proven narrowing policy must not emit the checked cast helper"
    );

    let driver = format!("console.log(JSON.stringify({}()));", lowered.function_name);
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(&analysed, &retained, "BigInt to Number narrowing");
    assert_eq!(analysed.stdout.trim(), r#"{"tag":"ok","value":1000}"#);
}

/// Verifies that a narrowing policy shared by a proven statement and an out-of-range statement
/// keeps its checked helpers while the proven statement still elides its own predicate.
#[test]
fn narrowing_policy_with_retained_use_keeps_checked_helpers() {
    let lowered = lower_numeric_fixture(
        "shared_narrow",
        NumericProfile::STANDARD,
        |_types, environment, region, expressions| {
            // The cast policy owns fixed I32 as its source domain, so both fixture literals carry
            // the actual I32 TypeId rather than the profile Int type.
            let i32_ty = builtin_type_ids::fixed_scalar(FixedScalar::I32);
            let carrier =
                fixture_carrier(environment, builtin_type_ids::fixed_scalar(FixedScalar::I8));
            let tuple_type = environment.intern_tuple(vec![carrier, carrier]);
            let (locals, local_names) =
                named_locals(&[(0, carrier, "safe"), (1, carrier, "unsafe")], region);
            NumericFixture {
                locals,
                local_names,
                statements: vec![
                    statement(
                        1,
                        narrowing_cast(
                            FixedScalar::I32,
                            FixedScalar::I8,
                            fixed_const(
                                FixedScalarValue::signed(FixedScalar::I32, 100)
                                    .expect("100 fits I32"),
                                i32_ty,
                                region,
                                expressions,
                            ),
                            0,
                        ),
                    ),
                    statement(
                        2,
                        narrowing_cast(
                            FixedScalar::I32,
                            FixedScalar::I8,
                            fixed_const(
                                FixedScalarValue::signed(FixedScalar::I32, 200)
                                    .expect("200 fits I32"),
                                i32_ty,
                                region,
                                expressions,
                            ),
                            1,
                        ),
                    ),
                ],
                return_value: expression(
                    HirExpressionKind::TupleConstruct {
                        elements: append_values(
                            &[
                                load_local(0, carrier, region, expressions),
                                load_local(1, carrier, region, expressions),
                            ],
                            expressions,
                        ),
                    },
                    tuple_type,
                    region,
                    ValueKind::RValue,
                    expressions,
                ),
            }
        },
    );

    assert!(
        lowered
            .analysed_table
            .integer_narrowing_is_safe(HirNodeId(1), NumericProfile::STANDARD),
        "the in-range narrowing must be proven"
    );
    assert!(
        !lowered
            .analysed_table
            .integer_narrowing_is_safe(HirNodeId(2), NumericProfile::STANDARD),
        "the out-of-range narrowing must stay checked"
    );
    assert!(
        lowered
            .analysed_source
            .contains("function __moth_cast_integer_to_integer("),
        "a retained use of the same policy must keep the checked cast helpers"
    );
    let driver = format!(
        "const results = {}();\nconsole.log(JSON.stringify(results[0]), __moth_error_code(results[1].value));",
        lowered.function_name
    );
    let (analysed, retained) = run_both_tables(&lowered, &driver);
    assert_same_runtime(&analysed, &retained, "shared narrowing policy");
    assert_eq!(
        analysed.stdout.trim(),
        format!(
            "{{\"tag\":\"ok\",\"value\":100}} {}",
            BuiltinErrorCode::IntCastOutOfRange.as_u32()
        ),
        "the proven use must return the exact safe carrier while the out-of-range use fails with the actual narrowing code"
    );
}

/// Verifies that exact U64 remainders stay on the BigInt carrier when proven.
#[test]
fn proven_u64_remainder_stays_on_bigint_carrier() {
    let lowered = lower_numeric_fixture(
        "u64_remainder",
        NumericProfile::STANDARD,
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
            .integer_operation_is_safe(HirNodeId(1), NumericProfile::STANDARD),
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
        NumericProfile::STANDARD,
        |types, _environment, region, expressions| {
            let (locals, local_names) = named_locals(
                &[
                    (0, types.int, "a"),
                    (1, types.int, "b"),
                    (2, types.int, "c"),
                    (3, types.int, "d"),
                    (4, types.int, "e"),
                    (5, types.int, "f"),
                ],
                region,
            );
            let int = types.int;
            NumericFixture {
                locals,
                local_names,
                statements: vec![
                    statement(
                        1,
                        define_local(0, int_expression(2, int, region, expressions)),
                    ),
                    statement(
                        2,
                        trap_op(
                            NumericOperator::Add,
                            NumericScalar::Int,
                            binary_operands(
                                load_local(0, int, region, expressions),
                                int_expression(3, int, region, expressions),
                            ),
                            1,
                        ),
                    ),
                    statement(
                        3,
                        trap_op(
                            NumericOperator::Multiply,
                            NumericScalar::Int,
                            binary_operands(
                                load_local(1, int, region, expressions),
                                int_expression(4, int, region, expressions),
                            ),
                            2,
                        ),
                    ),
                    statement(
                        4,
                        trap_op(
                            NumericOperator::Subtract,
                            NumericScalar::Int,
                            binary_operands(
                                load_local(2, int, region, expressions),
                                int_expression(5, int, region, expressions),
                            ),
                            3,
                        ),
                    ),
                    statement(
                        5,
                        trap_op(
                            NumericOperator::IntegerDivide,
                            NumericScalar::Int,
                            binary_operands(
                                load_local(3, int, region, expressions),
                                int_expression(2, int, region, expressions),
                            ),
                            4,
                        ),
                    ),
                    statement(
                        6,
                        trap_op(
                            NumericOperator::Remainder,
                            NumericScalar::Int,
                            binary_operands(
                                load_local(4, int, region, expressions),
                                int_expression(4, int, region, expressions),
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
