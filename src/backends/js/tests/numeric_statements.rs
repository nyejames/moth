//! Checked numeric operation lowering tests for JavaScript output.

use super::support::*;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::ids::TypeId;
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
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};
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
/// WHY: Float statement lowering tests need the same HIR scaffolding every time; keeping it in one
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
        float_expression(1.5, types.float, region, &mut expressions),
        types.string,
        expressions,
    );

    assert!(
        output.contains(
            "moth_result_l0 = __moth_binding(__moth_numeric_trap(__moth_format_float(1.5, 64, \"Float\")));"
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
        float_expression(1.5, types.float, region, &mut expressions),
        types.fallible_int_string,
        expressions,
    );

    assert!(
        output
            .contains("moth_result_l0 = __moth_binding(__moth_format_float(1.5, 64, \"Float\"));"),
        "ReturnError FormatFloat must assign the helper carrier directly"
    );
    assert!(
        !output.contains("__moth_numeric_trap(__moth_format_float"),
        "ReturnError FormatFloat must not wrap the helper in __moth_numeric_trap"
    );
}

/// Which value a Float boundary fixture returns from its entry function.
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

struct LoweredFloatBoundary {
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
    profile: NumericProfile,
    source_scalar: NumericScalar,
    lane: FloatBoundaryLane,
) -> LoweredFloatBoundary {
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
        .expect("Float boundary fixture must satisfy production HIR validation");

    let lowered = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        JsLoweringConfig::direct_js(false, profile),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("Float boundary fixture should lower to JavaScript");

    LoweredFloatBoundary {
        function_name: lowered
            .function_name_by_id
            .get(&FunctionId(0))
            .cloned()
            .expect("emitted boundary module carries its entry function name"),
        source: lowered.source,
    }
}

fn float32_profile() -> NumericProfile {
    NumericProfile {
        int_width: IntWidth::Bits32,
        float_precision: FloatPrecision::Bits32,
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

/// Verifies that a trapping boundary keeps a fixed F64 carrier exact under a Float32 profile.
///
/// WHAT: a finite high-range F64 value, the F64 successor of one and the minimum F64 subnormal
///       survive unchanged, signed zero stays signed, and every malformed or non-finite carrier
///       reports the shared Float boundary failure.
/// WHY: the validated source type owns the precision. Falling back to the module's Float32
///      profile would round the value to a different number, or overflow a finite F64 to infinity.
#[test]
fn trapping_float_boundary_keeps_fixed_f64_precision_under_a_float32_profile() {
    let lowered = lower_float_boundary_module(
        float32_profile(),
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
        float32_profile(),
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

/// Verifies that a fixed F32 boundary rounds once at its own precision under a Float64 profile.
///
/// WHAT: finite F64 carriers that are not F32 values round to the nearest F32, F32 subnormals
///       survive, a value that overflows the F32 finite range is rejected, and malformed carriers
///       are never coerced into numbers.
/// WHY: the module profile must not widen a fixed F32 boundary, and rounding before the finite
///      check is what turns a finite F64 overflow into the documented boundary failure.
#[test]
fn trapping_float_boundary_rounds_fixed_f32_once_under_a_float64_profile() {
    let lowered = lower_float_boundary_module(
        NumericProfile::STANDARD,
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

/// Verifies that a fixed F16 boundary completes exactly at binary16 under a Float64 profile.
///
/// WHAT: ties-to-even rounds `1 + 2^-11` to `1` while the representable `1 + 2^-10` survives, the
///       maximum finite `65504` is accepted, the minimum subnormal `2^-24` is preserved, and
///       `-(2^-25)` completes to negative zero. A finite `70000` overflows the F16 range and every
///       non-Number carrier is rejected without coercion.
/// WHY: the validated source type owns the precision, so an F16 boundary must complete at binary16
///      instead of leaving the wider module Float64 value untouched. Node 24+ is required for the
///      `Math.f16round` completion this lane demands.
#[test]
fn trapping_float_boundary_completes_fixed_f16_precision_exactly() {
    let lowered = lower_float_boundary_module(
        NumericProfile::STANDARD,
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

/// Verifies that the Float formatting helper is emitted when `FormatFloat` is reachable.
#[test]
fn format_float_helper_emitted_when_format_float_reachable() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_float_statement(
        HirFloatStatementKind::Format,
        NumericFailureMode::Trap,
        float_expression(1.5, types.float, region, &mut expressions),
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

/// Verifies that the Float validation helper is emitted when `ValidateFloat` is reachable.
#[test]
fn validate_float_helper_emitted_when_validate_float_reachable() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_float_statement(
        HirFloatStatementKind::Validate,
        NumericFailureMode::Trap,
        float_expression(1.5, types.float, region, &mut expressions),
        types.float,
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

/// Verifies that Float helpers are not emitted for modules without Float statements.
#[test]
fn float_helpers_not_emitted_without_float_statement() {
    let source = lower_minimal_module("main");

    assert!(
        !source.contains("function __moth_format_float("),
        "modules without Float statements must not emit __moth_format_float"
    );
    assert!(
        !source.contains("function __moth_float_validate("),
        "modules without Float statements must not emit __moth_float_validate"
    );
}

// Numeric operation statement lowering tests [numeric]
// ---------------------------------------------------------------------------

/// Builds and lowers a minimal module containing one `NumericOp` statement.
///
/// WHY: numeric lowering tests need the same HIR scaffolding every time; keeping it in one
/// helper lets each public fixture name only the operation, failure mode, operands, and result
/// type.
fn lower_minimal_module_with_numeric_op(
    op: HirNumericOp,
    failure_mode: NumericFailureMode,
    operands: HirNumericOperands,
    result_type: TypeId,
    expressions: HirExpressionStore,
) -> String {
    lower_minimal_module_with_numeric_op_for_profile(
        op,
        failure_mode,
        operands,
        result_type,
        NumericProfile::STANDARD,
        expressions,
    )
}

fn int_op(operator: NumericOperator) -> HirNumericOp {
    HirNumericOp {
        operator,
        domain: NumericScalar::Int,
    }
}

fn float_op(operator: NumericOperator) -> HirNumericOp {
    HirNumericOp {
        operator,
        domain: NumericScalar::Float,
    }
}

/// Verifies that trap-mode Int addition assigns the scalar success value to the result local.
#[test]
fn trap_mode_int_add_lowers_to_trapped_helper() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_numeric_op(
        int_op(NumericOperator::Add),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: int_expression(1, types.int, region, &mut expressions),
            right: int_expression(2, types.int, region, &mut expressions),
        },
        types.int,
        expressions,
    );

    assert!(
        output.contains(
            "moth_result_l0 = __moth_binding(__moth_numeric_trap(__moth_int_add(1, 2, -2147483648, 2147483647)));"
        ),
        "trap-mode Int addition must assign the checked Number carrier result"
    );
}

/// Verifies that return-error-mode Int addition assigns the fallible carrier directly.
#[test]
fn return_error_mode_int_add_lowers_to_carrier() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_numeric_op(
        int_op(NumericOperator::Add),
        NumericFailureMode::ReturnError,
        HirNumericOperands::Binary {
            left: int_expression(1, types.int, region, &mut expressions),
            right: int_expression(2, types.int, region, &mut expressions),
        },
        types.fallible_int_string,
        expressions,
    );

    assert!(
        output.contains(
            "moth_result_l0 = __moth_binding(__moth_int_add(1, 2, -2147483648, 2147483647));"
        ),
        "ReturnError Int addition must assign the helper carrier directly"
    );
    assert!(
        !output.contains("__moth_numeric_trap(__moth_int_add"),
        "ReturnError Int addition must not wrap the helper in __moth_numeric_trap"
    );
}

/// Verifies that a unary numeric operation lowers through the helper path.
#[test]
fn int_neg_lowers_to_unary_helper() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_numeric_op(
        int_op(NumericOperator::Negate),
        NumericFailureMode::Trap,
        HirNumericOperands::Unary {
            operand: int_expression(1, types.int, region, &mut expressions),
        },
        types.int,
        expressions,
    );

    assert!(
        output.contains(
            "moth_result_l0 = __moth_binding(__moth_numeric_trap(__moth_int_neg(1, -2147483648, 2147483647)));"
        ),
        "trap-mode Int negation must lower to the checked unary helper"
    );
}

/// Verifies that float operations also lower to the checked helper path.
#[test]
fn float_div_lowers_to_helper() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_numeric_op(
        float_op(NumericOperator::Divide),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: float_expression(1.0, types.float, region, &mut expressions),
            right: float_expression(2.0, types.float, region, &mut expressions),
        },
        types.float,
        expressions,
    );

    assert!(
        output.contains(
            "moth_result_l0 = __moth_binding(__moth_numeric_trap(__moth_float_div(1, 2)));"
        ),
        "trap-mode Float division must lower to the checked float helper"
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
        int_op(NumericOperator::Add),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: int_expression(1, types.int, region, &mut expressions),
            right: int_expression(2, types.int, region, &mut expressions),
        },
        types.int,
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
        "numeric operations must pass the semantic Int bounds to the shared helper"
    );
    assert!(
        !source.contains("function __moth_format_float("),
        "NumericOp should not emit the Float formatting helper"
    );
    assert!(
        !source.contains("function __moth_float_validate("),
        "NumericOp should not emit the Float boundary-validation helper"
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
        int_op(NumericOperator::Add),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: int_expression(1, types.int, region, &mut expressions),
            right: int_expression(2, types.int, region, &mut expressions),
        },
        types.int,
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

/// Verifies that integer helper successes normalize JS `-0` to the single Moth Int zero.
#[test]
fn int_ok_helper_normalizes_negative_zero() {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_numeric_op(
        int_op(NumericOperator::Negate),
        NumericFailureMode::Trap,
        HirNumericOperands::Unary {
            operand: int_expression(0, types.int, region, &mut expressions),
        },
        types.int,
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
        int_op(NumericOperator::Add),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: int_expression(1, types.int, region, &mut expressions),
            right: int_expression(2, types.int, region, &mut expressions),
        },
        types.int,
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

    // Int addition is binary but we supply unary operands.
    let numeric_statement = statement(
        1,
        HirStatementKind::NumericOp {
            op: int_op(NumericOperator::Add),
            failure_mode: NumericFailureMode::Trap,
            operands: HirNumericOperands::Unary {
                operand: int_expression(1, types.int, region, &mut expressions),
            },
            result: HirLocalDestination::Define(LocalId(0)),
        },
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, types.int, region)],
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
            op: int_op(NumericOperator::Add),
            failure_mode: NumericFailureMode::Trap,
            operands: HirNumericOperands::Binary {
                left: int_expression(1, types.int, region, &mut expressions),
                right: int_expression(2, types.int, region, &mut expressions),
            },
            result: HirLocalDestination::Update(LocalId(0)),
        },
    );
    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, types.int, region)],
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
