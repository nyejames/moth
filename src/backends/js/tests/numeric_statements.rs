//! Checked numeric operation lowering tests for JavaScript output.

use super::support::*;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::HirValueId;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode, RangeStepFailureCause,
};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirLocalDestination, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::validate_hir_module;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use std::process::Command;

#[test]
fn trap_mode_range_step_failure_lowers_to_fatal_throw() {
    for cause in [
        RangeStepFailureCause::ZeroStep,
        RangeStepFailureCause::NoProgress,
    ] {
        let mut expressions = HirExpressionStore::default();
        let mut string_table = StringTable::new();
        let mut path_fork = PathInternerFork::empty();
        let (type_environment, types) = build_type_environment();
        let region = RegionId(0);
        let block = HirBlock {
            id: BlockId(0),
            region,
            locals: vec![local(0, types.boolean, region)],
            statements: vec![statement(
                1,
                HirStatementKind::RangeStepFailure {
                    cause,
                    failure_mode: NumericFailureMode::Trap,
                    result: HirLocalDestination::Define(LocalId(0)),
                },
            )],
            terminator: HirTerminator::Return(unit_expression(
                types.unit,
                region,
                &mut expressions,
            )),
        };
        let function = HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![],
            return_type: types.unit,
        };
        let module = build_module(
            expressions,
            &mut path_fork,
            &mut string_table,
            "main",
            vec![block],
            function,
            &[(LocalId(0), "result")],
        );
        let output = lower_hir_to_js(
            &module,
            &NumericProofs::default(),
            &string_table,
            default_config(),
            &type_environment,
            &path_fork.snapshot_table(),
        )
        .expect("Trap-mode range failure must lower")
        .source;
        let code = cause.builtin_error_code();
        assert!(
            output.contains(&format!(
                "__moth_numeric_trap(__moth_error_result({:?}, {}))",
                code.default_message(),
                code.as_u32(),
            )),
            "fatal range guard must consume its typed cause through the numeric trap"
        );
        assert!(output.contains("throw new Error(__moth_error_message(carrier.value));"));
    }
}

// FormatFloat and ValidateFloat statement lowering tests [float]
// ---------------------------------------------------------------------------

/// Builds and lowers a minimal module containing one `FormatFloat` or `ValidateFloat` statement.
///
/// WHY: binary-float statement lowering tests need the same HIR scaffolding every time; keeping it in one
/// helper lets each public fixture name only the statement kind, failure mode, source, and result
/// type.
fn lower_minimal_module_with_float_statement(
    kind: HirFloatStatementKind,
    failure_mode: NumericFailureMode,
    source: HirValueId,
    result_type: TypeId,
    mut expressions: HirExpressionStore,
) -> String {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let statement_kind = match kind {
        HirFloatStatementKind::Format => HirStatementKind::FormatFloat {
            source,
            failure_mode,
            result: HirLocalDestination::Define(LocalId(0)),
        },
        HirFloatStatementKind::Validate => HirStatementKind::ValidateFloat {
            source,
            failure_mode,
            result: HirLocalDestination::Define(LocalId(0)),
        },
    };

    let float_statement = statement(1, statement_kind);

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, result_type, region)],
        statements: vec![float_statement],
        terminator: HirTerminator::Return(unit_expression(types.unit, region, &mut expressions)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "result")],
    );

    lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("JS lowering should succeed")
    .source
}

#[derive(Clone, Copy)]
enum HirFloatStatementKind {
    Format,
    Validate,
}

/// Verifies that trap-mode `FormatFloat` assigns the scalar formatted string to the result local.
#[test]
fn trap_mode_format_float_lowers_to_trapped_helper() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_float_statement(
        HirFloatStatementKind::Format,
        NumericFailureMode::Trap,
        f64_expression(1.5, types.f64, region, &mut expressions),
        types.string,
        expressions,
    );

    assert!(
        output.contains(
            "moth_result_l0 = __moth_binding(__moth_numeric_trap(__moth_format_float(1.5, 64, \"F64\")));"
        ),
        "trap-mode FormatFloat must assign the scalar trap result"
    );
}

/// Verifies that return-error-mode `FormatFloat` assigns the fallible carrier directly.
#[test]
fn return_error_mode_format_float_lowers_to_carrier() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_float_statement(
        HirFloatStatementKind::Format,
        NumericFailureMode::ReturnError,
        f64_expression(1.5, types.f64, region, &mut expressions),
        types.fallible_i32_string,
        expressions,
    );

    assert!(
        output.contains("moth_result_l0 = __moth_binding(__moth_format_float(1.5, 64, \"F64\"));"),
        "ReturnError FormatFloat must assign the helper carrier directly"
    );
    assert!(
        !output.contains("__moth_numeric_trap(__moth_format_float"),
        "ReturnError FormatFloat must not wrap the helper in __moth_numeric_trap"
    );
}

/// Which value a binary-float boundary fixture returns from its entry function.
///
/// WHY: trap and recoverable lowering differ only in the value the boundary hands back, so the
///      fixture shape stays shared while each lane names its observable contract.
#[derive(Clone, Copy)]
enum FloatBoundaryLane {
    /// Trap mode: the entry returns the validated scalar and throws on a rejected carrier.
    Scalar,
    /// Recoverable mode: the entry returns the internal fallible carrier for observation.
    Fallible,
}

/// One lowered entry: its emitted source and the generated name of function 0.
struct LoweredEntry {
    source: String,
    function_name: String,
}

/// Builds and lowers a module whose entry parameter is one raw external float carrier.
///
/// WHAT: the entry takes a raw carrier, validates it at the source type's exact precision, and
///       returns the scalar or the fallible carrier so a Node driver observes the real boundary.
/// WHY: non-finite, malformed and signed-zero carriers cannot exist as HIR literals, so boundary
///      behavior is only testable by executing the generated module with raw runtime values.
fn lower_float_boundary_module(
    source_scalar: NumericScalar,
    lane: FloatBoundaryLane,
) -> LoweredEntry {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (mut type_environment, _) = build_type_environment();
    let region = RegionId(0);

    let source_type = source_scalar.type_id(&type_environment);
    // The internal carrier carries the builtin `Error`, exactly like the carrier validated HIR
    // attaches to a ReturnError boundary result.
    let result_type = match lane {
        FloatBoundaryLane::Scalar => source_type,
        FloatBoundaryLane::Fallible => fixture_carrier(&mut type_environment, source_type),
    };
    let failure_mode = match lane {
        FloatBoundaryLane::Scalar => NumericFailureMode::Trap,
        FloatBoundaryLane::Fallible => NumericFailureMode::ReturnError,
    };
    let source = expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(0))),
        source_type,
        region,
        ValueKind::RValue,
        &mut expressions,
    );
    let returned = expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(1))),
        result_type,
        region,
        ValueKind::RValue,
        &mut expressions,
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, source_type, region), local(1, result_type, region)],
        statements: vec![statement(
            1,
            HirStatementKind::ValidateFloat {
                source,
                failure_mode,
                result: HirLocalDestination::Define(LocalId(1)),
            },
        )],
        terminator: HirTerminator::Return(returned),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![LocalId(0)],
        return_type: result_type,
    };
    let mut module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "value"), (LocalId(1), "result")],
    );

    // The fixture carries the entry-start tag production lowering records and passes the same HIR
    // validation every lowered module does, so it cannot drift into a shape real HIR rejects.
    module
        .function_origins
        .insert(FunctionId(0), HirFunctionOrigin::EntryStart);
    validate_hir_module(&module, &type_environment)
        .expect("binary-float boundary fixture must satisfy production HIR validation");

    let lowered = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        JsLoweringConfig::direct_js(false),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("binary-float boundary fixture should lower to JavaScript");

    LoweredEntry {
        function_name: lowered
            .function_name_by_id
            .get(&FunctionId(0))
            .cloned()
            .expect("emitted boundary module carries its entry function name"),
        source: lowered.source,
    }
}

/// Runs one generated JavaScript program through Node and returns its stdout.
///
/// WHY: exact boundary precision, rejection and signed-zero behavior are properties of the
///      executed module, not of the emitted source text.
fn run_javascript(source: &str) -> String {
    let output = Command::new("node")
        .args(["--eval", source])
        .output()
        .expect("Node.js is required for JavaScript runtime behavior tests");
    assert!(
        output.status.success(),
        "Node.js runtime failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("Node.js output is UTF-8")
}

/// Verifies that a trapping boundary keeps a fixed F64 carrier exact.
///
/// WHAT: a finite high-range F64 value, the F64 successor of one and the minimum F64 subnormal
///       survive unchanged, signed zero stays signed, and every malformed or non-finite carrier
///       reports the shared binary-float boundary failure.
/// WHY: the validated source type owns the precision. Falling back to a narrower F32
///      precision would round the value to another number, or overflow a finite F64 to infinity.
#[test]
fn trapping_float_boundary_keeps_fixed_f64_precision() {
    let lowered = lower_float_boundary_module(
        NumericScalar::Fixed(FixedScalar::F64),
        FloatBoundaryLane::Scalar,
    );
    let driver = format!(
        r#"
function attempt(value) {{
    try {{
        const produced = {function_name}(value);
        return "value:" + String(produced) + ":signedZero=" + Object.is(produced, -0);
    }} catch (error) {{
        return error instanceof Error ? "trap" : "threw:" + typeof error;
    }}
}}
let coercions = 0;
const hostileCarrier = {{ valueOf() {{ coercions += 1; return 0; }} }};
console.log([
    attempt(1e300),
    attempt(1.0000000000000002),
    attempt(5e-324),
    attempt(-0),
    attempt(NaN),
    attempt(Infinity),
    attempt(-Infinity),
    attempt("1.5"),
    attempt(true),
    attempt(undefined),
    attempt(1n),
    attempt({{}}),
    attempt(hostileCarrier),
    "coercions=" + coercions,
].join("\n"));
"#,
        function_name = lowered.function_name,
    );

    assert_eq!(
        run_javascript(&format!("{}\n{driver}", lowered.source)),
        "value:1e+300:signedZero=false\n\
         value:1.0000000000000002:signedZero=false\n\
         value:5e-324:signedZero=false\n\
         value:0:signedZero=true\n\
         trap\n\
         trap\n\
         trap\n\
         trap\n\
         trap\n\
         trap\n\
         trap\n\
         trap\n\
         trap\n\
         coercions=0\n",
        "an F64 boundary value must keep its 64-bit precision, reject every non-Number carrier \
         without coercing it, and terminate with an Error instead of returning a value"
    );
}

/// Verifies that a recoverable boundary reports code 304 exactly while keeping F64 precision.
///
/// WHY: the same finite and malformed carriers must produce the documented fallible Result on the
///      non-trapping route, so recovery callers branch on the error code rather than an exception.
#[test]
fn recoverable_float_boundary_reports_304_and_keeps_fixed_f64_precision() {
    let lowered = lower_float_boundary_module(
        NumericScalar::Fixed(FixedScalar::F64),
        FloatBoundaryLane::Fallible,
    );
    let boundary_code = BuiltinErrorCode::FloatBoundaryNonFinite.as_u32();
    let driver = format!(
        r#"
function attempt(value) {{
    let result;
    try {{
        result = {function_name}(value);
    }} catch (error) {{
        return error instanceof Error ? "trap" : "threw:" + typeof error;
    }}
    if (result.tag === "ok") {{
        return "ok:" + String(result.value) + ":signedZero=" + Object.is(result.value, -0);
    }}
    return "err:" + __moth_error_code(result.value)
        + ":tag=" + result.tag
        + ":message=" + (__moth_error_message(result.value).length > 0);
}}
let coercions = 0;
const hostileCarrier = {{ valueOf() {{ coercions += 1; return 0; }} }};
console.log([
    attempt(1e300),
    attempt(1.0000000000000002),
    attempt(5e-324),
    attempt(-0),
    attempt(NaN),
    attempt(Infinity),
    attempt(-Infinity),
    attempt("1.5"),
    attempt(true),
    attempt(undefined),
    attempt(1n),
    attempt({{}}),
    attempt(hostileCarrier),
    "coercions=" + coercions,
].join("\n"));
"#,
        function_name = lowered.function_name,
    );
    let rejected = format!("err:{boundary_code}:tag=err:message=true");

    assert_eq!(
        run_javascript(&format!("{}\n{driver}", lowered.source)),
        format!(
            "ok:1e+300:signedZero=false\n\
             ok:1.0000000000000002:signedZero=false\n\
             ok:5e-324:signedZero=false\n\
             ok:0:signedZero=true\n\
             {rejected}\n\
             {rejected}\n\
             {rejected}\n\
             {rejected}\n\
             {rejected}\n\
             {rejected}\n\
             {rejected}\n\
             {rejected}\n\
             {rejected}\n\
             coercions=0\n"
        ),
        "recoverable validation must keep F64 precision and report the exact boundary code"
    );
}

/// Verifies that a fixed F32 boundary rounds once at its own precision.
///
/// WHAT: finite F64 carriers that are not F32 values round to the nearest F32, F32 subnormals
///       survive, a value that overflows the F32 finite range is rejected, and malformed carriers
///       are never coerced into numbers.
/// WHY: a fixed F32 boundary must never be widened, and rounding before the finite
///      check is what turns a finite F64 overflow into the documented boundary failure.
#[test]
fn trapping_float_boundary_rounds_fixed_f32_once() {
    let lowered = lower_float_boundary_module(
        NumericScalar::Fixed(FixedScalar::F32),
        FloatBoundaryLane::Scalar,
    );
    let driver = format!(
        r#"
function attempt(value) {{
    try {{
        const produced = {function_name}(value);
        return "value:" + String(produced) + ":signedZero=" + Object.is(produced, -0);
    }} catch (error) {{
        return error instanceof Error ? "trap" : "threw:" + typeof error;
    }}
}}
let coercions = 0;
const hostileCarrier = {{ valueOf() {{ coercions += 1; return 0; }} }};
console.log([
    attempt(0.1),
    attempt(1 + 2 ** -24),
    attempt(2 ** -149),
    attempt(-(2 ** -150)),
    attempt(-0),
    attempt(3.5e38),
    attempt("0.1"),
    attempt(true),
    attempt(hostileCarrier),
    "coercions=" + coercions,
].join("\n"));
"#,
        function_name = lowered.function_name,
    );

    assert_eq!(
        run_javascript(&format!("{}\n{driver}", lowered.source)),
        "value:0.10000000149011612:signedZero=false\n\
         value:1:signedZero=false\n\
         value:1.401298464324817e-45:signedZero=false\n\
         value:0:signedZero=true\n\
         value:0:signedZero=true\n\
         trap\n\
         trap\n\
         trap\n\
         trap\n\
         coercions=0\n",
        "a fixed F32 boundary must round once at F32 precision, keep F32 subnormals and signed \
         zero, reject F32 overflow, and never coerce a non-Number carrier"
    );
}

/// Verifies that a fixed F16 boundary completes exactly at binary16.
///
/// WHAT: ties-to-even rounds `1 + 2^-11` to `1` while the representable `1 + 2^-10` survives, the
///       maximum finite `65504` is accepted, the minimum subnormal `2^-24` is preserved, and
///       `-(2^-25)` completes to negative zero. A finite `70000` overflows the F16 range and every
///       non-Number carrier is rejected without coercion.
/// WHY: the validated source type owns the precision, so an F16 boundary must complete at binary16
///      instead of leaving the wider module F64 value untouched. Node 24+ is required for the
///      `Math.f16round` completion this lane demands.
#[test]
fn trapping_float_boundary_completes_fixed_f16_precision_exactly() {
    let lowered = lower_float_boundary_module(
        NumericScalar::Fixed(FixedScalar::F16),
        FloatBoundaryLane::Scalar,
    );
    let driver = format!(
        r#"
function attempt(value) {{
    try {{
        const produced = {function_name}(value);
        return "value:" + String(produced) + ":signedZero=" + Object.is(produced, -0);
    }} catch (error) {{
        return error instanceof Error ? "trap" : "threw:" + typeof error;
    }}
}}
let coercions = 0;
const hostileCarrier = {{ valueOf() {{ coercions += 1; return 0; }} }};
console.log([
    attempt(1 + 2 ** -11),
    attempt(1 + 2 ** -10),
    attempt(65504),
    attempt(2 ** -24),
    attempt(-(2 ** -25)),
    attempt(70000),
    attempt("0.1"),
    attempt(true),
    attempt(hostileCarrier),
    "coercions=" + coercions,
].join("\n"));
"#,
        function_name = lowered.function_name,
    );

    assert_eq!(
        run_javascript(&format!("{}\n{driver}", lowered.source)),
        "value:1:signedZero=false\n\
         value:1.0009765625:signedZero=false\n\
         value:65504:signedZero=false\n\
         value:5.960464477539063e-8:signedZero=false\n\
         value:0:signedZero=true\n\
         trap\n\
         trap\n\
         trap\n\
         trap\n\
         coercions=0\n",
        "a fixed F16 boundary must complete at binary16, keep its maximum finite, subnormal and \
         signed-zero behavior, reject F16 overflow, and never coerce a non-Number carrier"
    );
}

/// Verifies that the binary-float formatting helper is emitted when `FormatFloat` is reachable.
#[test]
fn format_float_helper_emitted_when_format_float_reachable() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_float_statement(
        HirFloatStatementKind::Format,
        NumericFailureMode::Trap,
        f64_expression(1.5, types.f64, region, &mut expressions),
        types.string,
        expressions,
    );

    assert!(
        source.contains("function __moth_format_float("),
        "modules with FormatFloat must emit __moth_format_float"
    );
    assert!(
        !source.contains("function __moth_float_validate("),
        "FormatFloat should not emit the separate boundary-validation helper"
    );
    assert!(
        source.contains("function __moth_numeric_trap("),
        "modules with FormatFloat must emit __moth_numeric_trap"
    );
}

/// Verifies that the binary-float validation helper is emitted when `ValidateFloat` is reachable.
#[test]
fn validate_float_helper_emitted_when_validate_float_reachable() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_float_statement(
        HirFloatStatementKind::Validate,
        NumericFailureMode::Trap,
        f64_expression(1.5, types.f64, region, &mut expressions),
        types.f64,
        expressions,
    );

    assert!(
        source.contains("function __moth_float_validate("),
        "modules with ValidateFloat must emit __moth_float_validate"
    );
    assert!(
        !source.contains("function __moth_format_float("),
        "ValidateFloat should not emit the separate formatting helper"
    );
    assert!(
        source.contains("function __moth_numeric_trap("),
        "modules with ValidateFloat must emit __moth_numeric_trap"
    );
}

/// Verifies that binary-float helpers are not emitted for modules without binary-float statements.
#[test]
fn float_helpers_not_emitted_without_float_statement() {
    let source = lower_minimal_module("main");

    assert!(
        !source.contains("function __moth_format_float("),
        "modules without binary-float statements must not emit __moth_format_float"
    );
    assert!(
        !source.contains("function __moth_float_validate("),
        "modules without binary-float statements must not emit __moth_float_validate"
    );
}

// Numeric operation statement lowering tests [numeric]
// ---------------------------------------------------------------------------

fn i32_op(operator: NumericOperator) -> HirNumericOp {
    HirNumericOp {
        operator,
        domain: NumericScalar::Fixed(FixedScalar::I32),
    }
}

fn f64_op(operator: NumericOperator) -> HirNumericOp {
    HirNumericOp {
        operator,
        domain: NumericScalar::Fixed(FixedScalar::F64),
    }
}

/// Verifies that trap-mode I32 addition assigns the scalar success value to the result local.
#[test]
fn trap_mode_int_add_lowers_to_trapped_helper() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_numeric_op(
        i32_op(NumericOperator::Add),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: i32_expression(1, types.i32, region, &mut expressions),
            right: i32_expression(2, types.i32, region, &mut expressions),
        },
        types.i32,
        expressions,
    );

    assert!(
        output.contains(
            "moth_result_l0 = __moth_binding(__moth_numeric_trap(__moth_int_add(1, 2, -2147483648, 2147483647)));"
        ),
        "trap-mode I32 addition must assign the checked Number carrier result"
    );
}

/// Verifies that return-error-mode I32 addition assigns the fallible carrier directly.
#[test]
fn return_error_mode_int_add_lowers_to_carrier() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_numeric_op(
        i32_op(NumericOperator::Add),
        NumericFailureMode::ReturnError,
        HirNumericOperands::Binary {
            left: i32_expression(1, types.i32, region, &mut expressions),
            right: i32_expression(2, types.i32, region, &mut expressions),
        },
        types.fallible_i32_string,
        expressions,
    );

    assert!(
        output.contains(
            "moth_result_l0 = __moth_binding(__moth_int_add(1, 2, -2147483648, 2147483647));"
        ),
        "ReturnError I32 addition must assign the helper carrier directly"
    );
    assert!(
        !output.contains("__moth_numeric_trap(__moth_int_add"),
        "ReturnError I32 addition must not wrap the helper in __moth_numeric_trap"
    );
}

/// Verifies that a unary numeric operation lowers through the helper path.
#[test]
fn int_neg_lowers_to_unary_helper() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_numeric_op(
        i32_op(NumericOperator::Negate),
        NumericFailureMode::Trap,
        HirNumericOperands::Unary {
            operand: i32_expression(1, types.i32, region, &mut expressions),
        },
        types.i32,
        expressions,
    );

    assert!(
        output.contains(
            "moth_result_l0 = __moth_binding(__moth_numeric_trap(__moth_int_neg(1, -2147483648, 2147483647)));"
        ),
        "trap-mode I32 negation must lower to the checked unary helper"
    );
}

/// Verifies that float operations also lower to the checked helper path.
#[test]
fn float_div_lowers_to_helper() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_numeric_op(
        f64_op(NumericOperator::Divide),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: f64_expression(1.0, types.f64, region, &mut expressions),
            right: f64_expression(2.0, types.f64, region, &mut expressions),
        },
        types.f64,
        expressions,
    );

    assert!(
        output.contains(
            "moth_result_l0 = __moth_binding(__moth_numeric_trap(__moth_float_div(1, 2)));"
        ),
        "trap-mode F64 division must lower to the checked float helper"
    );
}

/// Verifies that numeric helpers are not emitted for modules without NumericOp.
#[test]
fn numeric_helpers_not_emitted_without_numeric_op() {
    let source = lower_minimal_module("main");

    assert!(
        !source.contains("function __moth_int_add("),
        "modules without NumericOp must not emit __moth_int_add"
    );
    assert!(
        !source.contains("function __moth_numeric_trap("),
        "modules without NumericOp must not emit __moth_numeric_trap"
    );
    assert!(
        !source.contains("function __moth_bigint_add("),
        "modules without a BigInteger-domain operation must not emit that helper family"
    );
}

/// Verifies that the numeric helper group is emitted when a NumericOp is reachable.
#[test]
fn numeric_helpers_emitted_when_numeric_op_reachable() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_numeric_op(
        i32_op(NumericOperator::Add),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: i32_expression(1, types.i32, region, &mut expressions),
            right: i32_expression(2, types.i32, region, &mut expressions),
        },
        types.i32,
        expressions,
    );

    assert!(
        source.contains("function __moth_int_add("),
        "numeric modules must emit __moth_int_add"
    );
    assert!(
        source.contains("function __moth_int_check("),
        "numeric modules must emit __moth_int_check"
    );
    assert!(
        source.contains("function __moth_numeric_trap("),
        "numeric modules must emit __moth_numeric_trap"
    );
    assert!(
        source.contains("__moth_int_add(1, 2, -2147483648, 2147483647)"),
        "numeric operations must pass the semantic I32 bounds to the shared helper"
    );
    assert!(
        !source.contains("function __moth_format_float("),
        "NumericOp should not emit the binary-float formatting helper"
    );
    assert!(
        !source.contains("function __moth_float_validate("),
        "NumericOp should not emit the binary-float boundary-validation helper"
    );
}

// Numeric helper contract tests [numeric-helper]
// ---------------------------------------------------------------------------

/// Verifies that the trap helper returns ok values and throws err values.
#[test]
fn numeric_trap_returns_ok_and_throws_err() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_numeric_op(
        i32_op(NumericOperator::Add),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: i32_expression(1, types.i32, region, &mut expressions),
            right: i32_expression(2, types.i32, region, &mut expressions),
        },
        types.i32,
        expressions,
    );

    let trap = helper_source(&source, "__moth_numeric_trap");

    assert!(
        trap.contains("carrier.tag === \"ok\"") && trap.contains("return carrier.value;"),
        "__moth_numeric_trap must return ok values"
    );
    assert!(
        trap.contains("carrier.tag === \"err\"")
            && trap.contains("throw new Error(__moth_error_message(carrier.value));"),
        "__moth_numeric_trap must throw JS errors using the canonical Moth error message"
    );
}

/// Verifies that integer helper successes normalise JS `-0` to the single Moth integer zero.
#[test]
fn int_ok_helper_normalizes_negative_zero() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_numeric_op(
        i32_op(NumericOperator::Negate),
        NumericFailureMode::Trap,
        HirNumericOperands::Unary {
            operand: i32_expression(0, types.i32, region, &mut expressions),
        },
        types.i32,
        expressions,
    );

    let helper = helper_source(&source, "__moth_int_ok");

    assert!(
        helper.contains("Object.is(value, -0) ? 0 : value"),
        "integer helpers must normalize JS -0 at the success boundary"
    );
}

/// Verifies that exact-Number integer helpers delegate range checking to `__moth_int_check`.
#[test]
fn int_helpers_delegate_to_int_check() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_numeric_op(
        i32_op(NumericOperator::Add),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: i32_expression(1, types.i32, region, &mut expressions),
            right: i32_expression(2, types.i32, region, &mut expressions),
        },
        types.i32,
        expressions,
    );

    let add = helper_source(&source, "__moth_int_add");

    assert!(
        add.contains("return __moth_int_check(a + b, min, max);"),
        "__moth_int_add must pass its semantic result range to __moth_int_check"
    );
    assert!(
        !add.contains("Number.isInteger(result)"),
        "__moth_int_add must not duplicate the integer carrier check"
    );
}

/// Verifies that malformed HIR arity produces a compiler error rather than invalid JS.
#[test]
fn numeric_op_arity_mismatch_returns_error() {
    let mut expressions = HirExpressionStore::default();
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    // I32 addition is binary but we supply unary operands.
    let numeric_statement = statement(
        1,
        HirStatementKind::NumericOp {
            op: i32_op(NumericOperator::Add),
            failure_mode: NumericFailureMode::Trap,
            operands: HirNumericOperands::Unary {
                operand: i32_expression(1, types.i32, region, &mut expressions),
            },
            result: HirLocalDestination::Define(LocalId(0)),
        },
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, types.i32, region)],
        statements: vec![numeric_statement],
        terminator: HirTerminator::Return(unit_expression(types.unit, region, &mut expressions)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "result")],
    );

    let result = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    );

    assert!(
        result.is_err(),
        "NumericOp arity mismatch must fail lowering with a compiler error"
    );
}

/// Verifies that an operation result explicitly updating an existing local uses ordinary
/// value-assignment semantics instead of allocating a fresh binding wrapper.
#[test]
fn numeric_op_update_uses_value_assignment() {
    let mut expressions = HirExpressionStore::default();
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let operation = statement(
        1,
        HirStatementKind::NumericOp {
            op: i32_op(NumericOperator::Add),
            failure_mode: NumericFailureMode::Trap,
            operands: HirNumericOperands::Binary {
                left: i32_expression(1, types.i32, region, &mut expressions),
                right: i32_expression(2, types.i32, region, &mut expressions),
            },
            result: HirLocalDestination::Update(LocalId(0)),
        },
    );
    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, types.i32, region)],
        statements: vec![operation],
        terminator: HirTerminator::Return(unit_expression(types.unit, region, &mut expressions)),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![LocalId(0)],
        return_type: types.unit,
    };
    let module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "value")],
    );

    let output = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("an explicit numeric operation update should lower");
    let value_name = expected_dev_local_name("value", 0);
    assert!(
        output.source.contains(&format!(
            "__moth_assign_value({value_name}, __moth_numeric_trap(__moth_int_add(1, 2"
        )),
        "an operation update must write its produced value through the existing binding"
    );
    assert!(
        !output.source.contains(&format!(
            "{value_name} = __moth_binding(__moth_numeric_trap(__moth_int_add(1, 2"
        )),
        "an operation update must not create a fresh value-backed binding"
    );
}

// D15 operation completion [numeric] [d15]
// ---------------------------------------------------------------------------

/// Builds and lowers one single-operation entry whose returned local carries the operation result.
///
/// WHY: D15 completion and narrow-width failure must be observed on the emitted operation rather
///      than by calling a runtime helper with hand-picked bounds, so the driver executes the
///      lowered function itself.
fn lower_returned_numeric_op(
    op: HirNumericOp,
    operands: HirNumericOperands,
    result_type: TypeId,
    mut expressions: HirExpressionStore,
) -> LoweredEntry {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (mut type_environment, _) = build_type_environment();
    let region = RegionId(0);

    // The entry recovers a rejected operation as the internal carrier, so the driver observes the
    // shared builtin error code instead of depending on trap wording.
    let carrier_type = fixture_carrier(&mut type_environment, result_type);
    let operation = statement(
        1,
        HirStatementKind::NumericOp {
            op,
            failure_mode: NumericFailureMode::ReturnError,
            operands,
            result: HirLocalDestination::Define(LocalId(0)),
        },
    );
    let returned = expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(0))),
        carrier_type,
        region,
        ValueKind::RValue,
        &mut expressions,
    );
    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, carrier_type, region)],
        statements: vec![operation],
        terminator: HirTerminator::Return(returned),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: carrier_type,
    };
    let mut module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "result")],
    );
    module
        .function_origins
        .insert(FunctionId(0), HirFunctionOrigin::EntryStart);
    validate_hir_module(&module, &type_environment)
        .expect("the D15 completion fixture must satisfy production HIR validation");

    let lowered = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("the D15 completion fixture should lower");

    LoweredEntry {
        function_name: lowered
            .function_name_by_id
            .get(&FunctionId(0))
            .cloned()
            .expect("the D15 fixture carries its entry function name"),
        source: lowered.source,
    }
}

/// The recovered outcome one emitted D15 operation must report.
///
/// WHY: the entry returns the internal carrier, so a completed operation is proved by its exact
///      scalar and signed zero while a rejected one is proved by the shared builtin error code;
///      neither depends on trap wording or emitted helper text.
#[derive(Clone, Copy)]
enum ExpectedOperation {
    /// The operation completed with this exact scalar rendering and signed-zero observation.
    Value {
        rendered: &'static str,
        signed_zero: bool,
    },
    /// The operation was rejected with this builtin error code.
    Error(BuiltinErrorCode),
}

impl ExpectedOperation {
    fn render(self) -> String {
        match self {
            Self::Value {
                rendered,
                signed_zero,
            } => format!("value:{rendered}:signedZero={signed_zero}"),
            Self::Error(code) => format!("err:{}", code.as_u32()),
        }
    }
}

/// Executes one lowered entry and renders the recovered carrier it returns.
///
/// WHY: the entry recovers failures as the internal carrier, so success is proved by the exact
///      returned scalar and failure by the shared builtin error code. An engine exception is
///      rendered as `threw:` and can never read as a numeric failure.
fn run_returned_numeric_op(entry: &LoweredEntry) -> String {
    let mut script = entry.source.clone();
    script.push_str(&format!(
        "\ntry {{\n\
         const produced = {}();\n\
         console.log(produced.tag === \"ok\" ? \"value:\" + String(produced.value) + \":signedZero=\" + Object.is(produced.value, -0) : produced.tag === \"err\" ? \"err:\" + __moth_error_code(produced.value) : \"invalid-carrier:\" + String(produced.tag));\n\
         }} catch (error) {{\n\
         console.log(\"threw:\" + (error instanceof Error ? error.name : typeof error));\n\
         }}",
        entry.function_name
    ));
    run_javascript(&script)
}

/// Verifies that an F16 operation completes at binary16 at each operation instead of widening.
///
/// WHAT: `1 + 2^-11` is exactly halfway between the two neighbouring F16 values, so ties-to-even
///       must complete it to 1; the finite maximum absorbs a step, `-1 * 0` and an underflowing
///       min-subnormal division keep signed zero, `2^-14 / 2` produces the first subnormal, and a
///       large product reports the shared non-finite failure code instead of succeeding at F32.
/// WHY: the old runtime promoted binary16 to binary32 before arithmetic, so a widened
///      intermediate would keep `1 + 2^-11` and change the selected domain's result.
#[test]
fn f16_operations_complete_at_binary16_at_each_operation() {
    let f16_ty = builtin_type_ids::fixed_scalar(FixedScalar::F16);
    let region = RegionId(0);

    for (operator, left, right, expected) in [
        (
            NumericOperator::Add,
            1.0,
            0.00048828125,
            ExpectedOperation::Value {
                rendered: "1",
                signed_zero: false,
            },
        ),
        (
            NumericOperator::Add,
            65504.0,
            1.0,
            ExpectedOperation::Value {
                rendered: "65504",
                signed_zero: false,
            },
        ),
        (
            NumericOperator::Multiply,
            -1.0,
            0.0,
            ExpectedOperation::Value {
                rendered: "0",
                signed_zero: true,
            },
        ),
        (
            NumericOperator::Divide,
            6.103515625e-5,
            2.0,
            ExpectedOperation::Value {
                rendered: "0.000030517578125",
                signed_zero: false,
            },
        ),
        (
            NumericOperator::Divide,
            -2.0_f64.powi(-24),
            2.0,
            ExpectedOperation::Value {
                rendered: "0",
                signed_zero: true,
            },
        ),
        (
            NumericOperator::Multiply,
            60000.0,
            60000.0,
            ExpectedOperation::Error(BuiltinErrorCode::FloatNonFinite),
        ),
    ] {
        let mut expressions = HirExpressionStore::default();
        let operands = HirNumericOperands::Binary {
            left: fixed_scalar_expression(
                FixedScalarValue::binary_float(FixedScalar::F16, left).expect("left fits F16"),
                f16_ty,
                region,
                &mut expressions,
            ),
            right: fixed_scalar_expression(
                FixedScalarValue::binary_float(FixedScalar::F16, right).expect("right fits F16"),
                f16_ty,
                region,
                &mut expressions,
            ),
        };
        let entry = lower_returned_numeric_op(
            HirNumericOp {
                operator,
                domain: NumericScalar::Fixed(FixedScalar::F16),
            },
            operands,
            f16_ty,
            expressions,
        );

        assert_eq!(
            run_returned_numeric_op(&entry).trim(),
            expected.render(),
            "the emitted F16 {operator:?} on {left} and {right} must complete at binary16"
        );
    }
}

/// Verifies that an I8 operation keeps the selected narrow width on the emitted operation.
///
/// WHAT: in-range addition and multiplication return their exact I8 results while the maximum and
///       product boundaries, the minimum subtraction and the minimum negation report the shared
///       integer overflow code instead of succeeding at a promoted 32-bit domain.
/// WHY: D15 preserves an equal-type narrow result, so a wider intermediate must not make an
///      overflowing I8 operation succeed.
#[test]
fn narrow_i8_operations_check_the_selected_width_at_each_operation() {
    let i8_ty = builtin_type_ids::fixed_scalar(FixedScalar::I8);
    let region = RegionId(0);

    for (operator, left, right, expected) in [
        (
            NumericOperator::Add,
            127,
            -1,
            ExpectedOperation::Value {
                rendered: "126",
                signed_zero: false,
            },
        ),
        (
            NumericOperator::Add,
            127,
            1,
            ExpectedOperation::Error(BuiltinErrorCode::IntOverflow),
        ),
        (
            NumericOperator::Subtract,
            -128,
            1,
            ExpectedOperation::Error(BuiltinErrorCode::IntOverflow),
        ),
        (
            NumericOperator::Multiply,
            11,
            11,
            ExpectedOperation::Value {
                rendered: "121",
                signed_zero: false,
            },
        ),
        (
            NumericOperator::Multiply,
            12,
            11,
            ExpectedOperation::Error(BuiltinErrorCode::IntOverflow),
        ),
    ] {
        let mut expressions = HirExpressionStore::default();
        let operands = HirNumericOperands::Binary {
            left: fixed_scalar_expression(
                FixedScalarValue::signed(FixedScalar::I8, left).expect("left fits I8"),
                i8_ty,
                region,
                &mut expressions,
            ),
            right: fixed_scalar_expression(
                FixedScalarValue::signed(FixedScalar::I8, right).expect("right fits I8"),
                i8_ty,
                region,
                &mut expressions,
            ),
        };
        let entry = lower_returned_numeric_op(
            HirNumericOp {
                operator,
                domain: NumericScalar::Fixed(FixedScalar::I8),
            },
            operands,
            i8_ty,
            expressions,
        );

        assert_eq!(
            run_returned_numeric_op(&entry).trim(),
            expected.render(),
            "the emitted I8 {operator:?} on {left} and {right} must use the I8 width"
        );
    }

    let mut expressions = HirExpressionStore::default();
    let operands = HirNumericOperands::Unary {
        operand: fixed_scalar_expression(
            FixedScalarValue::signed(FixedScalar::I8, -128).expect("-128 fits I8"),
            i8_ty,
            region,
            &mut expressions,
        ),
    };
    let entry = lower_returned_numeric_op(
        HirNumericOp {
            operator: NumericOperator::Negate,
            domain: NumericScalar::Fixed(FixedScalar::I8),
        },
        operands,
        i8_ty,
        expressions,
    );

    assert_eq!(
        run_returned_numeric_op(&entry).trim(),
        ExpectedOperation::Error(BuiltinErrorCode::IntOverflow).render(),
        "negating the I8 minimum must fail at I8 width with the shared overflow code"
    );
}
