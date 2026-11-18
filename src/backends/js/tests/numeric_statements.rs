//! Checked numeric operation lowering tests for JavaScript output.

use super::support::*;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::compiler_bug::CompilerBugReport;
use crate::compiler_frontend::datatypes::CallableReturnShape;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::number::NumberValue;
use crate::compiler_frontend::datatypes::numeric_operators::{NumericOperator, NumericSemantics};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expression_store::HirValueRange;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionReturnContract};
use crate::compiler_frontend::hir::ids::HirValueId;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId, LocalId, RegionId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, LoopStepFailureCause, NumericFailureMode,
};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirLocalDestination, HirStatementKind};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::hir::validate_hir_module;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use moth_lexical::numeric::grammar::NumericLiteralSign;

#[test]
fn trap_mode_loop_step_failure_lowers_to_fatal_throw() {
    for cause in [
        LoopStepFailureCause::ZeroStep,
        LoopStepFailureCause::NoProgress,
    ] {
        let expressions = HirExpressionStore::default();
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
                HirStatementKind::LoopStepFailure {
                    cause,
                    semantics: NumericSemantics::Ordinary,
                    result: HirLocalDestination::Define(LocalId(0)),
                },
            )],
            terminator: HirTerminator::Return(HirValueRange::empty()),
        };
        let function = HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![],
            return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
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
            "fatal loop-step guard must consume its typed cause through the numeric trap"
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
    expressions: HirExpressionStore,
) -> String {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, _) = build_type_environment();
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
        terminator: HirTerminator::Return(HirValueRange::empty()),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
            [],
            None,
        )),
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

/// Recoverable formatting delivers the scalar on the success edge, without a trap.
#[test]
fn recoverable_format_float_uses_native_success_channel() {
    let mut expressions = HirExpressionStore::default();
    let (_, types) = build_type_environment();
    let source = f64_expression(1.5, types.f64, RegionId(0), &mut expressions);
    let entry = lower_returned_operation(
        HirFallibleOperation::FormatFloat { source },
        types.string,
        expressions,
    );
    assert!(
        !entry
            .source
            .contains("__moth_numeric_trap(__moth_format_float")
    );
    let driver = format!("console.log(JSON.stringify({}()));", entry.function_name);
    assert_eq!(
        run_javascript(&format!("{}\n{driver}", entry.source)).trim(),
        r#"{"tag":"ok","value":"1.5"}"#
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
    /// Recoverable mode: native success/error returns are packed only at the JS ABI.
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
///       returns the scalar or ABI envelope so a Node driver observes the real boundary.
/// WHY: non-finite, malformed and signed-zero carriers cannot exist as HIR literals, so boundary
///      behavior is only testable by executing the generated module with raw runtime values.
fn lower_float_boundary_module(
    source_scalar: NumericScalar,
    lane: FloatBoundaryLane,
) -> LoweredEntry {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let source_type = source_scalar.type_id(&type_environment);
    let source = expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(0))),
        source_type,
        region,
        ValueKind::RValue,
        &mut expressions,
    );
    let returned = expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(1))),
        source_type,
        region,
        ValueKind::RValue,
        &mut expressions,
    );

    let blocks = match lane {
        FloatBoundaryLane::Scalar => vec![HirBlock {
            id: BlockId(0),
            region,
            locals: vec![local(0, source_type, region), local(1, source_type, region)],
            statements: vec![statement(
                1,
                HirStatementKind::ValidateFloat {
                    source,
                    failure_mode: NumericFailureMode::Trap,
                    result: HirLocalDestination::Define(LocalId(1)),
                },
            )],
            terminator: HirTerminator::Return(append_values(&[returned], &mut expressions)),
        }],
        FloatBoundaryLane::Fallible => invoke_return_blocks(
            HirFallibleOperation::ValidateFloat { source },
            source_type,
            types.error,
            vec![local(0, source_type, region)],
            1,
            &mut expressions,
        ),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![LocalId(0)],
        return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
            vec![source_type],
            match lane {
                FloatBoundaryLane::Scalar => None,
                FloatBoundaryLane::Fallible => Some(types.error),
            },
        )),
    };
    let module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        blocks,
        function,
        &[
            (LocalId(0), "value"),
            (LocalId(1), "result"),
            (LocalId(2), "error"),
        ],
    );

    validate_hir_module(&module, &type_environment, &ExternalPackageRegistry::new())
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

/// Proves the integer range candidate picks its JS carrier from HIR types, not runtime tags.
///
/// WHAT: an I32 counter with an I64 step must promote both Number operands with `BigInt(...)`
///       and convert the guarded successor back with `Number(...)`; two 32-bit operands stay on
///       exact Number arithmetic.
/// WHY: a `typeof` check would hide a carrier contradiction until runtime, and widening a 64-bit
///       value through Number would lose precision.
#[test]
fn integer_range_candidate_selects_static_bigint_carriers() {
    let promoted = integer_range_candidate_source(FixedScalar::I64);
    assert!(
        promoted.contains("BigInt(__range_candidate_current_"),
        "an I64 step must promote the I32 counter instead of widening through Number"
    );
    assert!(
        promoted.contains("BigInt(__range_candidate_end_"),
        "an I64 step must promote the bound instead of widening through Number"
    );
    assert!(
        promoted.contains("Number(__range_candidate_next_"),
        "an in-range successor must convert back to the I32 Number carrier"
    );
    assert!(
        emitted_candidate_line(&promoted, "__range_candidate_valid_").contains("0n"),
        "BigInt arithmetic must compare the step sign against a BigInt zero"
    );
    assert!(
        !promoted.contains("typeof __range_candidate"),
        "carrier selection must not dispatch on runtime value types"
    );

    let exact = integer_range_candidate_source(FixedScalar::I32);
    assert!(
        !exact.contains("BigInt(__range_candidate"),
        "two 32-bit operands must stay on exact Number arithmetic"
    );
    assert!(
        !exact.contains("Number(__range_candidate"),
        "an already Number-carried counter must not be converted back"
    );
    assert!(
        !emitted_candidate_line(&exact, "__range_candidate_valid_").contains("0n"),
        "Number arithmetic must compare the step sign against a Number zero"
    );
    assert!(
        !exact.contains("typeof __range_candidate"),
        "carrier selection must not dispatch on runtime value types"
    );
}

/// Lowers a module whose only statement is an I32-counter range candidate.
fn integer_range_candidate_source(step_scalar: FixedScalar) -> String {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let step_type = builtin_type_ids::fixed_scalar(step_scalar);

    let current = i32_expression(0, types.i32, region, &mut expressions);
    let step = fixed_scalar_expression(
        FixedScalarValue::signed(step_scalar, 3).expect("three fits every signed range step"),
        step_type,
        region,
        &mut expressions,
    );
    let end = i32_expression(10, types.i32, region, &mut expressions);

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, types.i32, region), local(1, types.boolean, region)],
        statements: vec![statement(
            1,
            HirStatementKind::RangeCandidate {
                current,
                step,
                end,
                inclusive: false,
                domain: NumericScalar::Fixed(FixedScalar::I32),
                candidate_result: HirLocalDestination::Define(LocalId(0)),
                in_range_result: HirLocalDestination::Define(LocalId(1)),
            },
        )],
        terminator: HirTerminator::Return(HirValueRange::empty()),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
            [],
            None,
        )),
    };
    let module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "candidate"), (LocalId(1), "in_range")],
    );

    lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("integer range candidates must lower")
    .source
}

fn emitted_candidate_line<'a>(source: &'a str, needle: &str) -> &'a str {
    source
        .lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("emitted JavaScript should contain a line with {needle}"))
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
    HirNumericOp::ordinary(operator, NumericScalar::Fixed(FixedScalar::I32))
}

fn f64_op(operator: NumericOperator) -> HirNumericOp {
    HirNumericOp::ordinary(operator, NumericScalar::Fixed(FixedScalar::F64))
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

/// Recoverable addition delivers its result through native edge-local definitions.
#[test]
fn recoverable_int_add_uses_native_success_channel() {
    let mut expressions = HirExpressionStore::default();
    let (_, types) = build_type_environment();
    let operands = HirNumericOperands::Binary {
        left: i32_expression(1, types.i32, RegionId(0), &mut expressions),
        right: i32_expression(2, types.i32, RegionId(0), &mut expressions),
    };
    let entry = lower_returned_numeric_op(
        i32_op(NumericOperator::Add),
        operands,
        types.i32,
        expressions,
    );
    assert!(!entry.source.contains("__moth_numeric_trap(__moth_int_add"));
    assert_eq!(
        run_returned_numeric_op(&entry).trim(),
        "value:3:signedZero=false"
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

/// Verifies that malformed HIR arity raises a compiler bug rather than emitting invalid JS.
#[test]
fn numeric_op_arity_mismatch_raises_compiler_bug() {
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
        terminator: HirTerminator::Return(HirValueRange::empty()),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
            [],
            None,
        )),
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

    let expected_message = format!(
        "JS backend received NumericOp::{} with operand arity that does not match the operation",
        i32_op(NumericOperator::Add)
    );
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = lower_hir_to_js(
            &module,
            &NumericProofs::default(),
            &string_table,
            default_config(),
            &type_environment,
            &path_fork.snapshot_table(),
        );
    }))
    .expect_err("NumericOp arity mismatch must raise a compiler bug");
    let report = payload
        .downcast::<CompilerBugReport>()
        .unwrap_or_else(|_| panic!("NumericOp arity mismatch must carry a CompilerBugReport"));
    assert_eq!(report.stage, CompilerStage::BackendLowering);
    assert_eq!(&*report.message, expected_message.as_str());
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
        terminator: HirTerminator::Return(HirValueRange::empty()),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![LocalId(0)],
        return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
            [],
            None,
        )),
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

// Operation completion [numeric]
// ---------------------------------------------------------------------------

/// Builds and lowers one single-operation entry whose returned local carries the operation result.
///
/// WHY: completion and narrow-width failure must be observed on the emitted operation rather
///      than by calling a runtime helper with hand-picked bounds, so the driver executes the
///      lowered function itself.
fn lower_returned_numeric_op(
    op: HirNumericOp,
    operands: HirNumericOperands,
    result_type: TypeId,
    expressions: HirExpressionStore,
) -> LoweredEntry {
    lower_returned_operation(
        HirFallibleOperation::NumericOp { op, operands },
        result_type,
        expressions,
    )
}

/// Edge locals carry semantic values; only Return/ReturnError pack the external ABI.
fn invoke_return_blocks(
    operation: HirFallibleOperation,
    result_type: TypeId,
    error_type: TypeId,
    mut locals: Vec<crate::compiler_frontend::hir::blocks::HirLocal>,
    result_id: u32,
    expressions: &mut HirExpressionStore,
) -> Vec<HirBlock> {
    let region = RegionId(0);
    let error_id = result_id + 1;
    locals.push(local(result_id, result_type, region));
    locals.push(local(error_id, error_type, region));
    let returned = expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(result_id))),
        result_type,
        region,
        ValueKind::RValue,
        expressions,
    );
    let error = expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(error_id))),
        error_type,
        region,
        ValueKind::RValue,
        expressions,
    );
    vec![
        HirBlock {
            id: BlockId(0),
            region,
            locals,
            statements: vec![],
            terminator: HirTerminator::Invoke {
                id: HirNodeId(1),
                span: None,
                operation,
                success: BlockId(1),
                success_results: vec![LocalId(result_id)],
                failure: BlockId(2),
                error_result: LocalId(error_id),
            },
        },
        HirBlock {
            id: BlockId(1),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(append_values(&[returned], expressions)),
        },
        HirBlock {
            id: BlockId(2),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::ReturnError(error),
        },
    ]
}

fn lower_returned_operation(
    operation: HirFallibleOperation,
    result_type: TypeId,
    expressions: HirExpressionStore,
) -> LoweredEntry {
    let (type_environment, types) = build_type_environment();
    lower_returned_operation_in_environment(
        operation,
        result_type,
        expressions,
        type_environment,
        types.error,
    )
}

fn lower_returned_operation_in_environment(
    operation: HirFallibleOperation,
    result_type: TypeId,
    mut expressions: HirExpressionStore,
    type_environment: TypeEnvironment,
    error_type: TypeId,
) -> LoweredEntry {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let blocks = invoke_return_blocks(
        operation,
        result_type,
        error_type,
        vec![],
        0,
        &mut expressions,
    );
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
            vec![result_type],
            Some(error_type),
        )),
    };
    let module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        blocks,
        function,
        &[(LocalId(0), "result"), (LocalId(1), "error")],
    );
    validate_hir_module(&module, &type_environment, &ExternalPackageRegistry::new())
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

/// Builds and lowers one NumericOp statement whose result is the entry's scalar return value.
///
/// WHAT: the entry returns the statement result directly, without a fallible ABI envelope, so a
///       Node driver observes the exact emitted value and carrier representation.
/// WHY: total numeric operations no longer call a helper, so only executing the lowered statement
///      proves that Dec coefficients, half-even rounding and signed float zero are unchanged.
fn lower_statement_scalar_result(
    failure_mode: NumericFailureMode,
    build: impl FnOnce(
        &mut TypeEnvironment,
        RegionId,
        &mut HirExpressionStore,
    ) -> (HirNumericOp, HirNumericOperands, TypeId),
) -> ScalarEntry {
    let (mut type_environment, _) = build_type_environment();
    let region = RegionId(0);
    let mut expressions = HirExpressionStore::default();
    let (op, operands, result_type) = build(&mut type_environment, region, &mut expressions);
    lower_returned_scalar_entry(
        type_environment,
        expressions,
        HirStatementKind::NumericOp {
            op,
            failure_mode,
            operands,
            result: HirLocalDestination::Define(LocalId(0)),
        },
        result_type,
    )
}

/// Executes one scalar-returning entry and renders the driver expression.
fn run_scalar_result(entry: &ScalarEntry, render: &str) -> String {
    let mut script = entry.source.clone();
    script.push_str(&format!(
        "\nconst value = {}();\nconsole.log({render});\n",
        entry.function_name
    ));
    run_javascript(&script)
}

/// One Dec expression written as normalized decimal text and a sign.
type DecOperand = (&'static str, bool);

/// Builds one exact Dec literal in the caller's already-interned Number domain.
fn dec_expression(
    (text, negative): DecOperand,
    scale: NumberScale,
    ty: TypeId,
    region: RegionId,
    expressions: &mut HirExpressionStore,
) -> HirValueId {
    let sign = if negative {
        NumericLiteralSign::Negative
    } else {
        NumericLiteralSign::Positive
    };
    let value = NumberValue::from_normalized(text, sign, scale)
        .expect("test Dec operand fits its requested scale");
    expression(
        HirExpressionKind::Number(value),
        ty,
        region,
        ValueKind::Const,
        expressions,
    )
}

/// Builds and lowers one ordinary Dec statement over exact literals at `scale`.
///
/// WHAT: `right` selects the binary operand pair or a unary operand, and both operands are built
///       in the caller's already-interned Number domain.
/// WHY: the total-operation tests vary only operator, delivery and operand pair, so one fixture
///      keeps their HIR shape and scalar return shape identical.
fn lower_dec_statement(
    failure_mode: NumericFailureMode,
    operator: NumericOperator,
    scale: NumberScale,
    left: DecOperand,
    right: Option<DecOperand>,
) -> ScalarEntry {
    lower_statement_scalar_result(failure_mode, |environment, region, expressions| {
        let number_type = environment.intern_number(scale);
        let operands = match right {
            Some(right) => HirNumericOperands::Binary {
                left: dec_expression(left, scale, number_type, region, expressions),
                right: dec_expression(right, scale, number_type, region, expressions),
            },
            None => HirNumericOperands::Unary {
                operand: dec_expression(left, scale, number_type, region, expressions),
            },
        };
        (
            HirNumericOp::ordinary(operator, NumericScalar::Number(scale)),
            operands,
            number_type,
        )
    })
}

/// Total Dec add, subtract, negate and multiply emit exact BigInt coefficients with no carrier.
///
/// WHAT: runs both `Trap` and `Infallible` statement delivery over runtime-shaped operands and
///       observes the generated coefficient and `bigint` carrier at a positive and a zero scale.
/// WHY: a carrier keeping `{ tag: "ok" }` or a positive-scale multiply that skipped the shared
///      half-even kernel would return a wrong value, while a scale-zero multiply is exact
///      coefficient arithmetic and must not demand that kernel. A deleted helper reference would
///      fail in Node.
#[test]
fn total_dec_operations_lower_to_exact_scalar_coefficients() {
    let scale_two = NumberScale::new(2).expect("scale two is valid");
    let cases = [
        (
            scale_two,
            NumericOperator::Add,
            ("0.05", false),
            Some(("0.50", false)),
            "55",
            "__moth_binding((5n + 50n))",
        ),
        (
            scale_two,
            NumericOperator::Subtract,
            ("0.15", false),
            Some(("0.50", false)),
            "-35",
            "__moth_binding((15n - 50n))",
        ),
        (
            scale_two,
            NumericOperator::Negate,
            ("0.05", false),
            None,
            "-5",
            "__moth_binding((-(5n)))",
        ),
        (
            scale_two,
            NumericOperator::Multiply,
            ("0.05", false),
            Some(("0.50", false)),
            "2",
            "__moth_binding(__moth_number_round_half_even((5n * 50n), 100n))",
        ),
        (
            scale_two,
            NumericOperator::Multiply,
            ("0.15", false),
            Some(("0.50", false)),
            "8",
            "__moth_binding(__moth_number_round_half_even((15n * 50n), 100n))",
        ),
        (
            scale_two,
            NumericOperator::Multiply,
            ("0.05", true),
            Some(("0.50", false)),
            "-2",
            "__moth_binding(__moth_number_round_half_even((-5n * 50n), 100n))",
        ),
        (
            scale_two,
            NumericOperator::Multiply,
            ("0.15", true),
            Some(("0.50", false)),
            "-8",
            "__moth_binding(__moth_number_round_half_even((-15n * 50n), 100n))",
        ),
        (
            NumberScale::ZERO,
            NumericOperator::Multiply,
            ("5", false),
            Some(("50", false)),
            "250",
            "__moth_binding((5n * 50n))",
        ),
        (
            NumberScale::ZERO,
            NumericOperator::Multiply,
            ("5", true),
            Some(("50", false)),
            "-250",
            "__moth_binding((-5n * 50n))",
        ),
    ];

    for (scale, operator, left, right, expected_coefficient, expected_expression) in cases {
        for failure_mode in [NumericFailureMode::Trap, NumericFailureMode::Infallible] {
            let entry = lower_dec_statement(failure_mode, operator, scale, left, right);

            assert!(
                entry.source.contains(expected_expression),
                "{operator:?} at scale {} in {failure_mode:?} delivery must lower to \
                 {expected_expression}:\n{}",
                scale.get(),
                entry.source
            );
            assert!(
                !entry.source.contains("__moth_numeric_trap("),
                "{operator:?} in {failure_mode:?} delivery must not consume a numeric carrier"
            );
            for deleted in [
                "function __moth_number_add(",
                "function __moth_number_sub(",
                "function __moth_number_mul(",
                "function __moth_number_neg(",
            ] {
                assert!(
                    !entry.source.contains(deleted),
                    "{operator:?} must not emit the deleted helper {deleted}"
                );
            }
            let expects_kernel = operator == NumericOperator::Multiply && scale.get() > 0;
            assert_eq!(
                entry
                    .source
                    .contains("function __moth_number_round_half_even("),
                expects_kernel,
                "{operator:?} at scale {} kernel demand is wrong",
                scale.get()
            );

            assert_eq!(
                run_scalar_result(&entry, "typeof value + \":\" + String(value)").trim(),
                format!("bigint:{expected_coefficient}"),
                "{operator:?} at scale {} in {failure_mode:?} delivery must keep an exact BigInt \
                 coefficient",
                scale.get()
            );
        }
    }
}

/// Total Dec negation keeps coefficients beyond Number's exact integer range.
///
/// WHAT: negates a scale-zero coefficient far beyond 2^53 and observes the exact BigInt value.
/// WHY: the removed checked negation helper covered this magnitude; the scalar path must not
///      widen through a JS Number or normalise the sign incorrectly when the carrier is BigInt.
#[test]
fn total_dec_negation_keeps_large_coefficients_exact() {
    let scale = NumberScale::ZERO;
    let coefficient = "123456789012345678901234567890";
    let entry = lower_dec_statement(
        NumericFailureMode::Trap,
        NumericOperator::Negate,
        scale,
        (coefficient, false),
        None,
    );

    assert!(
        entry
            .source
            .contains("__moth_binding((-(123456789012345678901234567890n)))"),
        "large-coefficient negation must stay exact BigInt arithmetic:\n{}",
        entry.source
    );
    assert!(!entry.source.contains("__moth_numeric_trap("));
    assert_eq!(
        run_scalar_result(&entry, "typeof value + \":\" + String(value)").trim(),
        "bigint:-123456789012345678901234567890"
    );
}

/// Total binary-float negation emits one scalar, keeps signed zero and demands no float family.
#[test]
fn total_float_negation_lowers_to_scalar_and_preserves_signed_zero() {
    let cases = [
        (FixedScalar::F64, 0.0_f64, "(-(0))", "-0"),
        (FixedScalar::F64, 1.5, "(-(1.5))", "-1.5"),
        (FixedScalar::F32, 0.0, "Math.fround(-(0))", "-0"),
        (FixedScalar::F32, 1.5, "Math.fround(-(1.5))", "-1.5"),
        (FixedScalar::F16, 0.0, "Math.f16round(-(0))", "-0"),
        (FixedScalar::F16, 1.5, "Math.f16round(-(1.5))", "-1.5"),
    ];

    for (scalar, input, expected_expression, expected_value) in cases {
        let ty = builtin_type_ids::fixed_scalar(scalar);
        for failure_mode in [NumericFailureMode::Trap, NumericFailureMode::Infallible] {
            let entry = lower_statement_scalar_result(failure_mode, |_, region, expressions| {
                let value = FixedScalarValue::binary_float(scalar, input)
                    .expect("test binary float fits its declared precision");
                let operand = fixed_scalar_expression(value, ty, region, expressions);
                (
                    HirNumericOp::ordinary(NumericOperator::Negate, NumericScalar::Fixed(scalar)),
                    HirNumericOperands::Unary { operand },
                    ty,
                )
            });

            assert!(
                entry
                    .source
                    .contains(&format!("__moth_binding({expected_expression})")),
                "{scalar:?} negation in {failure_mode:?} delivery must lower to {expected_expression}:\n{}",
                entry.source
            );
            assert!(
                !entry.source.contains("__moth_numeric_trap("),
                "{scalar:?} negation in {failure_mode:?} delivery must not consume a numeric carrier"
            );
            for family in ["float", "float32", "float16"] {
                for operation in ["add", "sub", "mul", "div", "mod", "pow", "neg"] {
                    let helper = format!("function __moth_{family}_{operation}(");
                    assert!(
                        !entry.source.contains(&helper),
                        "{scalar:?} negation must not emit the checked helper {helper}"
                    );
                }
            }

            assert_eq!(
                run_scalar_result(
                    &entry,
                    "typeof value + \":\" + (Object.is(value, -0) ? \"-0\" : String(value))"
                )
                .trim(),
                format!("number:{expected_value}"),
                "{scalar:?} negation in {failure_mode:?} delivery must preserve the exact value and signed zero"
            );
        }
    }
}

/// Total operations store their scalar success without a failure edge.
///
/// WHAT: drives Dec multiply and F64 negation through a fallible entry and observes the scalar
///       value, BigInt/Number carrier and signed zero from the returned ABI envelope.
/// WHY: the Invoke path must not read `.value` from a scalar, stage a capture temp, demand a trap
///      helper, or retain a dead `if (true)` branch and error block for an operation that cannot
///      fail; the entry's declared `Error!` return envelope stays unchanged.
#[test]
fn total_operations_transfer_scalar_success_without_a_failure_edge() {
    let scale = NumberScale::new(2).expect("scale two is valid");
    let (mut type_environment, types) = build_type_environment();
    let number_type = type_environment.intern_number(scale);
    let region = RegionId(0);
    let mut expressions = HirExpressionStore::default();
    let left = dec_expression(
        ("0.05", false),
        scale,
        number_type,
        region,
        &mut expressions,
    );
    let right = dec_expression(
        ("0.50", false),
        scale,
        number_type,
        region,
        &mut expressions,
    );
    let entry = lower_returned_operation_in_environment(
        HirFallibleOperation::NumericOp {
            op: HirNumericOp::ordinary(NumericOperator::Multiply, NumericScalar::Number(scale)),
            operands: HirNumericOperands::Binary { left, right },
        },
        number_type,
        expressions,
        type_environment,
        types.error,
    );
    assert!(
        !entry.source.contains("function __moth_number_mul("),
        "a total Dec multiply must not emit the deleted multiply helper"
    );
    assert!(
        !entry.source.contains("__moth_numeric_trap("),
        "a total Dec Invoke must store its scalar directly"
    );
    assert!(
        entry
            .source
            .contains("__moth_number_round_half_even((5n * 50n), 100n)"),
        "a total Dec multiply must call the shared half-even kernel"
    );
    assert!(
        entry
            .source
            .contains("= __moth_binding(__moth_number_round_half_even((5n * 50n), 100n));"),
        "the Dec Invoke must store the scalar expression directly in its success slot"
    );
    assert!(
        !entry.source.contains("const __invoke_result_"),
        "a total numeric Invoke must not stage its scalar through a capture temp"
    );
    assert!(
        !entry.source.contains(".tag === \"ok\""),
        "a total numeric Invoke must not test its scalar result as a carrier"
    );
    assert!(
        !entry.source.contains("if (true)"),
        "a total numeric Invoke must not emit a dead success conditional"
    );
    assert!(
        !entry.source.contains("tag: \"err\", value: __moth_read("),
        "a total numeric Invoke's failure block must not be emitted:\n{}",
        entry.source
    );
    assert!(
        entry.source.contains("return { tag: \"ok\", value:"),
        "the entry's declared Error! return envelope must remain"
    );
    assert_eq!(
        run_returned_numeric_op(&entry).trim(),
        "value:2:signedZero=false"
    );

    let f64_ty = builtin_type_ids::fixed_scalar(FixedScalar::F64);
    let mut float_expressions = HirExpressionStore::default();
    let entry = lower_returned_numeric_op(
        HirNumericOp::ordinary(
            NumericOperator::Negate,
            NumericScalar::Fixed(FixedScalar::F64),
        ),
        HirNumericOperands::Unary {
            operand: f64_expression(0.0, f64_ty, region, &mut float_expressions),
        },
        f64_ty,
        float_expressions,
    );
    assert!(
        !entry.source.contains("function __moth_float_neg("),
        "a total F64 negation must not emit the deleted checked negation helper"
    );
    assert!(
        !entry.source.contains("__moth_numeric_trap("),
        "a total F64 Invoke must store its scalar directly"
    );
    assert!(
        entry.source.contains("= __moth_binding((-(0)));"),
        "the F64 Invoke must store the scalar negation expression directly"
    );
    assert!(
        !entry.source.contains(".tag === \"ok\""),
        "a total numeric Invoke must not test its scalar result as a carrier"
    );
    assert!(
        !entry.source.contains("if (true)"),
        "a total numeric Invoke must not emit a dead success conditional"
    );
    assert!(
        !entry.source.contains("tag: \"err\", value: __moth_read("),
        "a total numeric Invoke's failure block must not be emitted:\n{}",
        entry.source
    );
    assert!(
        entry.source.contains("return { tag: \"ok\", value:"),
        "the entry's declared Error! return envelope must remain"
    );
    assert_eq!(
        run_returned_numeric_op(&entry).trim(),
        "value:0:signedZero=true"
    );
}

/// The recovered outcome one emitted operation must report.
///
/// WHY: the entry returns an ABI envelope, so a completed operation is proved by its exact
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

/// Executes one lowered entry and renders its success/error ABI envelope.
///
/// WHY: the entry exposes native success/error channels at its ABI, so success is proved by the exact
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
/// WHY: promoting binary16 to binary32 before arithmetic would keep a widened
///      intermediate of `1 + 2^-11` and change the selected domain's result.
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
            HirNumericOp::ordinary(operator, NumericScalar::Fixed(FixedScalar::F16)),
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
/// WHY: an equal-type narrow result is preserved, so a wider intermediate must not make an
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
            HirNumericOp::ordinary(operator, NumericScalar::Fixed(FixedScalar::I8)),
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
        HirNumericOp::ordinary(
            NumericOperator::Negate,
            NumericScalar::Fixed(FixedScalar::I8),
        ),
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
