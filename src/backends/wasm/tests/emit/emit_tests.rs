use crate::backends::error_types::BackendErrorType;
use crate::backends::wasm::backend::lower_hir_to_wasm_module;
use crate::backends::wasm::emit::module::emit_lir_to_wasm_module;
use crate::backends::wasm::emit::sections::{
    build_emit_plan, helper_emit_order, helper_name, helper_signature,
};
use crate::backends::wasm::lir::function::{WasmLirBlock, WasmLirFunction, WasmLirFunctionOrigin};
use crate::backends::wasm::lir::instructions::{
    IntegerCheckMode, WasmCalleeRef, WasmCheckedIntegerOperation, WasmIntegerOperationKind,
    WasmIntegerPowerScratch, WasmIntegerScratch, WasmLirStmt, WasmLirTerminator,
    WasmNumericOperationOperands,
};
use crate::backends::wasm::lir::linkage::{
    WasmExport, WasmExportKind, WasmFunctionLinkage, WasmImport, WasmImportKind,
};
use crate::backends::wasm::lir::module::{WasmLirModule, WasmStaticData, WasmStaticDataKind};
use crate::backends::wasm::lir::types::{
    WasmAbiType, WasmImportId, WasmLirBlockId, WasmLirFunctionId, WasmLirLocal, WasmLirLocalId,
    WasmLirSignature, WasmLocalRole, WasmStaticDataId,
};
use crate::backends::wasm::request::{
    WasmBackendRequest, WasmCfgLoweringStrategy, WasmDebugFlags, WasmEmitOptions, WasmExportPolicy,
    WasmHelperExportPolicy, WasmTargetFeatures,
};
use crate::backends::wasm::runtime::memory::{WasmMemoryPlan, WasmScalarStorageKind};
use crate::backends::wasm::runtime::strings::WasmRuntimeHelper;
use crate::backends::wasm::tests::lowering::test_support::{
    build_module, build_type_environment, default_borrow_facts, default_numeric_proofs, expression,
    int_expression, load_local as hir_load_local, local as hir_local, statement as hir_statement,
    string_expression,
};
use crate::compiler_frontend::analysis::numeric_proofs::analyse_numeric_proofs;
use crate::compiler_frontend::compiler_messages::compiler_errors::ErrorType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{
    HirExpressionKind, HirVariantCarrier, HirVariantField, ValueKind,
};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::operators::HirBinOp;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::{HirAssertionMessageEvaluation, HirTerminator};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::binary16::round_f64_to_f16;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::format::format_finite_float;
use moth_lexical::numeric::precision::BinaryFloatPrecision;
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};
use rustc_hash::FxHashMap;
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn lowers_hir_to_wasm_module_bytes() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let start_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");

    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(1, 7, types.int, RegionId(0))),
    };
    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.int,
    };

    let hir_module = build_module(
        &mut path_fork,
        &mut string_table,
        vec![(start_function, start_path, HirFunctionOrigin::EntryStart)],
        vec![start_block],
        FunctionId(0),
    );

    let mut request = WasmBackendRequest::default();
    request.emit_options.validate_emitted_module = false;
    let result = lower_hir_to_wasm_module(
        &hir_module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &request,
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("Wasm lowering should emit module bytes");
    let wasm_bytes = result.wasm_bytes.expect("wasm bytes should be available");
    validate_wasm(&wasm_bytes);
    let output = run_wasm_node_script(
        &wasm_bytes,
        "",
        r#"
const module = new WebAssembly.Module(bytes);
const assertions = WebAssembly.Module.imports(module).filter(
  item => item.module === "host" && item.name === "assertion_failed"
);
process.stdout.write(JSON.stringify(assertions));
"#,
    );
    assert_eq!(
        String::from_utf8(output).expect("Node import list should be UTF-8"),
        "[]",
        "assertion-free output must not demand the assertion host import"
    );
}

#[test]
fn emitted_assertions_deliver_static_messages_to_host_before_trapping_in_node() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (mut type_environment, types) = build_type_environment();
    let option_string = type_environment.intern_option(types.string);
    let cases = [
        ("default_message", None),
        ("folded_message", Some("folded 🦋\nmessage")),
        ("empty_message", Some("")),
    ];
    let mut functions = Vec::new();
    let mut blocks = Vec::new();
    let mut request = WasmBackendRequest::default();
    request.export_policy.helper_exports = WasmHelperExportPolicy {
        export_memory: true,
        export_str_ptr: true,
        export_str_len: true,
        export_release: true,
        ..Default::default()
    };

    for (index, (name, text)) in cases.into_iter().enumerate() {
        let function_id = FunctionId(index as u32);
        let block_id = BlockId(index as u32);
        let region = RegionId(0);
        let path = path_fork
            .try_intern_portable_path(name, &mut string_table)
            .expect("test path fits");
        let (variant_index, fields, message_evaluation) = match text {
            None => (0, vec![], HirAssertionMessageEvaluation::Default),
            Some(text) => (
                1,
                vec![HirVariantField {
                    name: None,
                    value: string_expression(index as u32 * 2 + 1, text, types.string, region),
                }],
                HirAssertionMessageEvaluation::Folded,
            ),
        };
        blocks.push(HirBlock {
            id: block_id,
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::AssertFailure {
                message: expression(
                    index as u32 * 2,
                    HirExpressionKind::VariantConstruct {
                        carrier: HirVariantCarrier::Option,
                        variant_index,
                        fields,
                    },
                    option_string,
                    region,
                    ValueKind::Const,
                ),
                message_evaluation,
            },
        });
        functions.push((
            HirFunction {
                id: function_id,
                entry: block_id,
                params: vec![],
                return_type: types.unit,
            },
            path,
            if index == 0 {
                HirFunctionOrigin::EntryStart
            } else {
                HirFunctionOrigin::Normal
            },
        ));
        request.export_policy.exported_functions.push(function_id);
        request
            .export_policy
            .export_names
            .insert(function_id, name.to_owned());
    }

    let module = build_module(
        &mut path_fork,
        &mut string_table,
        functions,
        blocks,
        FunctionId(0),
    );
    let result = lower_hir_to_wasm_module(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &request,
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("static assertion HIR should emit valid Wasm");
    let wasm_bytes = result.wasm_bytes.expect("Wasm bytes should be present");
    const NODE_SETUP: &str = r#"
const assert = require("node:assert/strict");
assert.deepEqual(WebAssembly.Module.imports(new WebAssembly.Module(bytes)), [
  { module: "host", name: "assertion_failed", kind: "function" }
]);
let assertionInstance;
let throwFromHost = false;
let hostError;
const events = [];
imports.host = {
  assertion_failed(handle) {
    const exports = assertionInstance.exports;
    const pointer = exports.moth_str_ptr(handle);
    const length = exports.moth_str_len(handle);
    const message = new TextDecoder().decode(
      new Uint8Array(exports.memory.buffer, pointer, length)
    );
    exports.moth_release(handle);
    events.push(message);
    if (throwFromHost) {
      hostError = new Error(message);
      Object.defineProperty(hostError, "__moth_assertion", { value: true });
      throw hostError;
    }
  }
};
"#;
    const NODE_BODY: &str = r#"
assertionInstance = instance;
for (const [name, expected] of [
  ["default_message", "assertion failed"],
  ["folded_message", "folded 🦋\nmessage"],
  ["empty_message", ""]
]) {
  events.length = 0;
  throwFromHost = false;
  assert.throws(() => instance.exports[name](), error => {
    assert(error instanceof WebAssembly.RuntimeError);
    assert.match(error.message, /unreachable/);
    events.push("trap");
    return true;
  });
  assert.deepEqual(events, [expected, "trap"]);

  events.length = 0;
  throwFromHost = true;
  assert.throws(() => instance.exports[name](), error => {
    assert.strictEqual(error, hostError);
    assert.equal(error.message, expected);
    const marker = Object.getOwnPropertyDescriptor(error, "__moth_assertion");
    assert.equal(marker.value, true);
    assert.equal(marker.enumerable, false);
    events.push("host-error");
    return true;
  });
  assert.deepEqual(events, [expected, "host-error"]);
}
process.stdout.write("assertion-import-ok");
"#;
    let output = run_wasm_node_script(&wasm_bytes, NODE_SETUP, NODE_BODY);
    assert_eq!(output, b"assertion-import-ok");
}

#[test]
fn emits_requested_helper_exports_and_valid_section_order() {
    let lir_module = build_manual_lir_module();
    let request = request_with_helper_exports();

    let emit_result =
        emit_lir_to_wasm_module(&lir_module, &request).expect("manual lir emission should succeed");
    validate_wasm(&emit_result.wasm_bytes);

    let exports = collect_export_names(&emit_result.wasm_bytes);
    assert!(exports.contains(&"memory".to_owned()));
    assert!(exports.contains(&"moth_str_ptr".to_owned()));
    assert!(exports.contains(&"moth_str_len".to_owned()));
    assert!(exports.contains(&"moth_release".to_owned()));
    assert!(exports.contains(&"moth_call_0".to_owned()));

    let order = collect_section_order(&emit_result.wasm_bytes);
    assert_order(&order, "type", "import");
    assert_order(&order, "import", "function");
    assert_order(&order, "function", "memory");
    assert_order(&order, "memory", "global");
    assert_order(&order, "global", "export");
    assert_order(&order, "export", "code");
    assert_order(&order, "code", "data");
}

#[test]
fn rejects_invalid_helper_export_policy() {
    let mut request = WasmBackendRequest::default();
    request.export_policy.helper_exports = WasmHelperExportPolicy {
        export_memory: true,
        export_str_ptr: true,
        export_str_len: false,
        export_vec_new: false,
        export_vec_push: false,
        export_vec_len: false,
        export_vec_get: false,
        export_release: false,
    };

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let start_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");
    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(1, 0, types.int, RegionId(0))),
    };
    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.int,
    };
    let hir_module = build_module(
        &mut path_fork,
        &mut string_table,
        vec![(start_function, start_path, HirFunctionOrigin::EntryStart)],
        vec![start_block],
        FunctionId(0),
    );

    let error = lower_hir_to_wasm_module(
        &hir_module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &request,
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect_err("invalid helper policy should fail");
    let error = error
        .infrastructure_error()
        .expect("Wasm generation failure should be wrapped for rendering");
    assert_eq!(
        &error.error_type,
        &ErrorType::Backend(BackendErrorType::WasmGeneration)
    );
    assert!(error.msg.contains("moth_str_ptr"));
}

#[test]
fn rejects_mismatched_checked_integer_operand_types() {
    let mut module = build_manual_lir_module();
    let main = module
        .functions
        .iter_mut()
        .find(|function| function.id == WasmLirFunctionId(1))
        .expect("manual main function should be present");

    main.blocks[0]
        .statements
        .push(WasmLirStmt::CheckedIntegerOp {
            operation: WasmCheckedIntegerOperation {
                dst: WasmLirLocalId(13),
                operator: NumericOperator::Add,
                kind: WasmIntegerOperationKind::Signed64,
                check_mode: IntegerCheckMode::Runtime,
                operands: WasmNumericOperationOperands::Binary {
                    left: WasmLirLocalId(1),
                    right: WasmLirLocalId(3),
                },
                scratch: WasmIntegerScratch {
                    product: None,
                    power: None,
                },
            },
        });

    let error = emit_lir_to_wasm_module(&module, &WasmBackendRequest::default())
        .expect_err("mismatched checked integer operands should fail emission");
    assert_eq!(
        error.error_type,
        ErrorType::Backend(BackendErrorType::WasmGeneration)
    );
}

#[test]
fn checked_integer_machine_boundaries_execute_with_native_traps() {
    let case_names = [
        "u64_mul_zero",
        "u64_mul_overflow",
        "i32_mul_overflow_high",
        "i32_mul_overflow_low",
        "i64_mul_underflow_min_times_two",
        "i64_mul_underflow_two_times_min",
        "i32_div_min_by_negative_one",
        "i64_add_overflow_high",
        "i64_add_overflow_low",
        "i64_sub_overflow_low",
        "i64_sub_overflow_high",
        "i64_power_preserves_sources",
    ];
    let module = build_checked_integer_boundary_module();
    let result = emit_lir_to_wasm_module(&module, &WasmBackendRequest::default())
        .expect("checked integer boundary module should emit");
    validate_wasm(&result.wasm_bytes);
    let export_specs = case_names
        .iter()
        .map(|name| (*name, None))
        .collect::<Vec<_>>();
    let actual = execute_wasm_checked_exports_in_node(&result.wasm_bytes, &export_specs);
    let expected = [
        ("u64_mul_zero", "ok", "0"),
        ("u64_mul_overflow", "trap", "unreachable"),
        ("i32_mul_overflow_high", "trap", "unreachable"),
        ("i32_mul_overflow_low", "trap", "unreachable"),
        ("i64_mul_underflow_min_times_two", "trap", "unreachable"),
        ("i64_mul_underflow_two_times_min", "trap", "unreachable"),
        ("i32_div_min_by_negative_one", "trap", "unreachable"),
        ("i64_add_overflow_high", "trap", "unreachable"),
        ("i64_add_overflow_low", "trap", "unreachable"),
        ("i64_sub_overflow_low", "trap", "unreachable"),
        ("i64_sub_overflow_high", "trap", "unreachable"),
        ("i64_power_preserves_sources", "ok", "7"),
    ];

    assert_eq!(actual.len(), expected.len());
    for ((actual_name, actual_status, actual_value), (name, status, value)) in
        actual.iter().zip(expected)
    {
        assert_eq!(actual_name.as_str(), name);
        assert_eq!(
            actual_status.as_str(),
            status,
            "{name} had an unexpected outcome"
        );
        if status == "trap" {
            assert!(
                actual_value.contains(value),
                "{name} should trap through Wasm unreachable, found {actual_value:?}"
            );
        } else {
            assert_eq!(
                actual_value.as_str(),
                value,
                "{name} returned an unexpected result"
            );
        }
    }
}

#[test]
fn checked_float_machine_boundaries_execute_with_native_traps() {
    let cases = [
        checked_float_binary_case(
            "f32_divide_zero",
            BinaryFloatPrecision::Binary32,
            NumericOperator::Divide,
            1.0,
            0.0,
        ),
        checked_float_binary_case(
            "f32_remainder_zero",
            BinaryFloatPrecision::Binary32,
            NumericOperator::Remainder,
            1.0,
            0.0,
        ),
        checked_float_binary_case(
            "f32_add_overflow",
            BinaryFloatPrecision::Binary32,
            NumericOperator::Add,
            f32::MAX as f64,
            f32::MAX as f64,
        ),
        checked_float_binary_case(
            "f32_subtract_overflow",
            BinaryFloatPrecision::Binary32,
            NumericOperator::Subtract,
            f32::MAX as f64,
            -(f32::MAX as f64),
        ),
        checked_float_binary_case(
            "f32_multiply_overflow",
            BinaryFloatPrecision::Binary32,
            NumericOperator::Multiply,
            f32::MAX as f64,
            2.0,
        ),
        checked_float_binary_case(
            "f32_finite_f64_demotes_to_infinity",
            BinaryFloatPrecision::Binary32,
            NumericOperator::Power,
            2.0,
            128.0,
        ),
        checked_float_binary_case(
            "f32_power_domain",
            BinaryFloatPrecision::Binary32,
            NumericOperator::Power,
            -2.0,
            0.5,
        ),
        checked_float_unary_case(
            "f32_negate_nan",
            BinaryFloatPrecision::Binary32,
            NumericOperator::Negate,
            f64::NAN,
        ),
        checked_float_binary_case(
            "f64_divide_zero",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Divide,
            1.0,
            0.0,
        ),
        checked_float_binary_case(
            "f64_remainder_zero",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Remainder,
            1.0,
            0.0,
        ),
        checked_float_binary_case(
            "f64_add_overflow",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Add,
            f64::MAX,
            f64::MAX,
        ),
        checked_float_binary_case(
            "f64_subtract_overflow",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Subtract,
            f64::MAX,
            -f64::MAX,
        ),
        checked_float_binary_case(
            "f64_multiply_overflow",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Multiply,
            f64::MAX,
            2.0,
        ),
        checked_float_binary_case(
            "f64_power_overflow",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Power,
            2.0,
            1024.0,
        ),
        checked_float_binary_case(
            "f64_power_domain",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Power,
            -2.0,
            0.5,
        ),
        checked_float_unary_case(
            "f64_negate_nan",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Negate,
            f64::NAN,
        ),
    ];
    let module = build_checked_float_operation_module(&cases);
    let result = emit_lir_to_wasm_module(&module, &WasmBackendRequest::default())
        .expect("checked float boundary module should emit");
    validate_wasm(&result.wasm_bytes);
    let export_specs = cases
        .iter()
        .map(|case| (case.name, Some(case.precision)))
        .collect::<Vec<_>>();
    let actual = execute_wasm_checked_exports_in_node(&result.wasm_bytes, &export_specs);

    assert_eq!(actual.len(), cases.len());
    for ((actual_name, actual_status, actual_value), case) in actual.iter().zip(cases) {
        assert_eq!(actual_name.as_str(), case.name);
        assert_eq!(actual_status, "trap", "{} should trap", case.name);
        assert!(
            actual_value.contains("unreachable"),
            "{} should trap through Wasm unreachable, found {actual_value:?}",
            case.name
        );
    }
}

#[test]
fn validate_float_preserves_finite_bits_and_traps_nonfinite_values_in_node() {
    let module = build_float_validation_lir_module();
    let result = emit_lir_to_wasm_module(&module, &WasmBackendRequest::default())
        .expect("Float validation LIR should emit");
    validate_wasm(&result.wasm_bytes);

    let f32_inputs = [
        "7f7fffff",         // largest finite
        "00000001",         // smallest subnormal
        "80000000",         // negative zero
        "7fc00000",         // NaN
        "7f800000",         // positive infinity
        "ff800000",         // negative infinity
        "7fefffffffffffff", // finite F64 rounds to positive F32 infinity at the boundary
        "ffefffffffffffff", // finite F64 rounds to negative F32 infinity at the boundary
    ];
    let f64_inputs = [
        "7fefffffffffffff", // largest finite
        "0000000000000001", // smallest subnormal
        "8000000000000000", // negative zero
        "7ff8000000000000", // NaN
        "7ff0000000000000", // positive infinity
        "fff0000000000000", // negative infinity
    ];
    let export_specs: [(&str, BinaryFloatPrecision, &[&str]); 4] = [
        (
            "validate_f32_distinct",
            BinaryFloatPrecision::Binary32,
            &f32_inputs,
        ),
        (
            "validate_f32_alias",
            BinaryFloatPrecision::Binary32,
            &f32_inputs,
        ),
        (
            "validate_f64_distinct",
            BinaryFloatPrecision::Binary64,
            &f64_inputs,
        ),
        (
            "validate_f64_alias",
            BinaryFloatPrecision::Binary64,
            &f64_inputs,
        ),
    ];
    let actual = execute_wasm_float_validation_in_node(&result.wasm_bytes, &export_specs);
    assert_eq!(
        actual.len(),
        export_specs
            .iter()
            .map(|(_, _, inputs)| inputs.len())
            .sum::<usize>(),
    );

    let mut actual_rows = actual.iter();
    for (name, precision, inputs) in export_specs {
        for input_bits in inputs {
            let actual = actual_rows
                .next()
                .expect("Node should report every validation result");
            let mut fields = actual.splitn(4, '|');
            assert_eq!(fields.next(), Some(name));
            assert_eq!(fields.next(), Some(*input_bits));
            let status = fields.next().expect("Node result should include status");
            let value = fields
                .next()
                .expect("Node result should include bits or trap");

            let finite = match precision {
                BinaryFloatPrecision::Binary32 if input_bits.len() == 16 => {
                    (f64::from_bits(u64::from_str_radix(input_bits, 16).unwrap()) as f32)
                        .is_finite()
                }
                BinaryFloatPrecision::Binary32 => {
                    f32::from_bits(u32::from_str_radix(input_bits, 16).unwrap()).is_finite()
                }
                BinaryFloatPrecision::Binary64 => {
                    f64::from_bits(u64::from_str_radix(input_bits, 16).unwrap()).is_finite()
                }
                BinaryFloatPrecision::Binary16 => {
                    panic!("Float validation tests require F32 or F64")
                }
            };
            if finite {
                assert_eq!(status, "ok", "{name} should accept {input_bits}");
                assert_eq!(
                    value, *input_bits,
                    "{name} should preserve the exact finite bits for {input_bits}"
                );
            } else {
                assert_eq!(status, "trap", "{name} should trap for {input_bits}");
                assert!(
                    value.contains("unreachable"),
                    "{name} should trap through Wasm unreachable, found {value:?}"
                );
            }
        }
    }
}

#[test]
fn checked_float_remainders_and_f32_arithmetic_return_exact_bits() {
    let cases = [
        checked_float_binary_case(
            "f64_remainder_max_mod_3",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Remainder,
            f64::MAX,
            3.0,
        ),
        checked_float_binary_case(
            "f64_remainder_max_mod_min_subnormal",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Remainder,
            f64::MAX,
            f64::from_bits(1),
        ),
        checked_float_binary_case(
            "f64_remainder_negative_exact_is_negative_zero",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Remainder,
            -6.0,
            3.0,
        ),
        checked_float_binary_case(
            "f64_remainder_subnormal",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Remainder,
            f64::from_bits(3),
            f64::from_bits(2),
        ),
        checked_float_binary_case(
            "f64_remainder_negative_divisor_larger_magnitude",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Remainder,
            -0.5,
            -2.0,
        ),
        checked_float_binary_case(
            "f32_add_half_ulp_rounds_to_even",
            BinaryFloatPrecision::Binary32,
            NumericOperator::Add,
            1.0,
            2.0_f64.powi(-24),
        ),
        checked_float_binary_case(
            "f32_add_subnormals",
            BinaryFloatPrecision::Binary32,
            NumericOperator::Add,
            f32::from_bits(1) as f64,
            f32::from_bits(1) as f64,
        ),
        checked_float_binary_case(
            "f32_remainder_negative_exact_is_negative_zero",
            BinaryFloatPrecision::Binary32,
            NumericOperator::Remainder,
            -6.0,
            3.0,
        ),
        checked_float_binary_case(
            "f32_multiply_negative_zero",
            BinaryFloatPrecision::Binary32,
            NumericOperator::Multiply,
            -0.0,
            2.0,
        ),
    ];
    let expected = [
        ("f64_remainder_max_mod_3", "4000000000000000"),
        ("f64_remainder_max_mod_min_subnormal", "0000000000000000"),
        (
            "f64_remainder_negative_exact_is_negative_zero",
            "8000000000000000",
        ),
        ("f64_remainder_subnormal", "0000000000000001"),
        (
            "f64_remainder_negative_divisor_larger_magnitude",
            "bfe0000000000000",
        ),
        ("f32_add_half_ulp_rounds_to_even", "3f800000"),
        ("f32_add_subnormals", "00000002"),
        ("f32_remainder_negative_exact_is_negative_zero", "80000000"),
        ("f32_multiply_negative_zero", "80000000"),
    ];
    let module = build_checked_float_operation_module(&cases);
    let result = emit_lir_to_wasm_module(&module, &WasmBackendRequest::default())
        .expect("checked float successful boundary module should emit");
    validate_wasm(&result.wasm_bytes);
    let export_specs = cases
        .iter()
        .map(|case| (case.name, Some(case.precision)))
        .collect::<Vec<_>>();
    let actual = execute_wasm_checked_exports_in_node(&result.wasm_bytes, &export_specs);

    assert_eq!(actual.len(), expected.len());
    for ((actual_name, actual_status, actual_bits), (expected_name, expected_bits)) in
        actual.iter().zip(expected)
    {
        assert_eq!(actual_name.as_str(), expected_name);
        assert_eq!(actual_status, "ok", "{expected_name} should succeed");
        assert_eq!(
            actual_bits, expected_bits,
            "{expected_name} returned unexpected float bits"
        );
    }
}

#[test]
fn checked_float_power_boundaries_return_exact_bits_or_bounded_near_e() {
    let cases = [
        checked_float_binary_case(
            "f64_power_two_to_min_subnormal",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Power,
            2.0,
            -1074.0,
        ),
        checked_float_binary_case(
            "f64_power_negative_base_odd",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Power,
            -2.0,
            3.0,
        ),
        checked_float_binary_case(
            "f64_power_negative_base_even",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Power,
            -2.0,
            4.0,
        ),
        checked_float_binary_case(
            "f64_power_zero_exponent",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Power,
            2.0,
            0.0,
        ),
        checked_float_binary_case(
            "f64_power_zero_to_zero",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Power,
            0.0,
            0.0,
        ),
        checked_float_binary_case(
            "f64_power_zero_base",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Power,
            0.0,
            3.0,
        ),
        checked_float_binary_case(
            "f64_power_negative_zero_odd",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Power,
            -0.0,
            3.0,
        ),
        checked_float_binary_case(
            "f64_power_negative_zero_even",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Power,
            -0.0,
            2.0,
        ),
        checked_float_binary_case(
            "f64_power_huge_exponent_underflows_to_zero",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Power,
            0.5,
            f64::MAX,
        ),
        checked_float_binary_case(
            "f64_power_huge_even_exponent_underflows_to_positive_zero",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Power,
            -0.5,
            f64::MAX,
        ),
        checked_float_binary_case(
            "f64_power_huge_odd_exponent_underflows_to_negative_zero",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Power,
            -0.5,
            2049.0,
        ),
        checked_float_binary_case(
            "f32_power_two_to_min_subnormal",
            BinaryFloatPrecision::Binary32,
            NumericOperator::Power,
            2.0,
            -149.0,
        ),
        checked_float_binary_case(
            "f32_power_negative_base_odd",
            BinaryFloatPrecision::Binary32,
            NumericOperator::Power,
            -2.0,
            3.0,
        ),
        checked_float_binary_case(
            "f64_power_near_one_approaches_e",
            BinaryFloatPrecision::Binary64,
            NumericOperator::Power,
            1.0 + 2.0_f64.powi(-52),
            2.0_f64.powi(52),
        ),
    ];
    let exact_expected = [
        ("f64_power_two_to_min_subnormal", "0000000000000001"),
        ("f64_power_negative_base_odd", "c020000000000000"),
        ("f64_power_negative_base_even", "4030000000000000"),
        ("f64_power_zero_exponent", "3ff0000000000000"),
        ("f64_power_zero_to_zero", "3ff0000000000000"),
        ("f64_power_zero_base", "0000000000000000"),
        ("f64_power_negative_zero_odd", "8000000000000000"),
        ("f64_power_negative_zero_even", "0000000000000000"),
        (
            "f64_power_huge_exponent_underflows_to_zero",
            "0000000000000000",
        ),
        (
            "f64_power_huge_even_exponent_underflows_to_positive_zero",
            "0000000000000000",
        ),
        (
            "f64_power_huge_odd_exponent_underflows_to_negative_zero",
            "8000000000000000",
        ),
        ("f32_power_two_to_min_subnormal", "00000001"),
        ("f32_power_negative_base_odd", "c1000000"),
    ];
    let module = build_checked_float_operation_module(&cases);
    let result = emit_lir_to_wasm_module(&module, &WasmBackendRequest::default())
        .expect("checked float power boundary module should emit");
    validate_wasm(&result.wasm_bytes);
    let export_specs = cases
        .iter()
        .map(|case| (case.name, Some(case.precision)))
        .collect::<Vec<_>>();
    let actual = execute_wasm_checked_exports_in_node(&result.wasm_bytes, &export_specs);

    assert_eq!(actual.len(), cases.len());
    for ((actual_name, actual_status, actual_bits), (expected_name, expected_bits)) in
        actual.iter().zip(exact_expected)
    {
        assert_eq!(actual_name, expected_name);
        assert_eq!(actual_status, "ok", "{expected_name} should succeed");
        assert_eq!(
            actual_bits, expected_bits,
            "{expected_name} returned unexpected float bits"
        );
    }
    let (actual_name, actual_status, actual_bits) = &actual[exact_expected.len()];
    assert_eq!(actual_name.as_str(), "f64_power_near_one_approaches_e");
    assert_eq!(actual_status, "ok");
    let actual_near_e_bits =
        u64::from_str_radix(actual_bits, 16).expect("near-e float bits should be hexadecimal");
    let e_bits = std::f64::consts::E.to_bits();
    // For h = 2^-52, (1 + h)^(1 / h) = e * (1 - h/2 + O(h^2)), less
    // than one ULP from e. The emitted fdlibm helper is nearly, not
    // universally, correctly rounded, so allow a bounded distance rather
    // than pinning a host-libm result bitwise.
    assert!(
        actual_near_e_bits.abs_diff(e_bits) <= 4,
        "near-one power was {actual_near_e_bits:016x}, more than four ULP from e ({e_bits:016x})"
    );
}

#[test]
fn checked_f64_power_matches_wasm_javascript_and_rust_bits() {
    let mut cases = vec![
        // Prior backend mismatches: a negative non-integer base with an even exponent and a
        // near-one base raised to a large positive exponent.
        (0xbff1_9999_9999_999a, 0x4010_0000_0000_0000),
        (0x3ff0_0000_0000_0001, 0x4330_0000_0000_0000),
        // Odd/even negative integer powers and signed-zero behavior.
        (0xc000_0000_0000_0000, 0x4008_0000_0000_0000),
        (0xc000_0000_0000_0000, 0x4010_0000_0000_0000),
        (0x0000_0000_0000_0000, 0x4008_0000_0000_0000),
        (0x8000_0000_0000_0000, 0x4008_0000_0000_0000),
        (0x8000_0000_0000_0000, 0x4000_0000_0000_0000),
        (0x4000_0000_0000_0000, (-0.0_f64).to_bits()),
        // Subnormal input normalization and results, signed underflow, and finite near-overflow.
        (0x4000_0000_0000_0000, (-1074.0_f64).to_bits()),
        (0x0000_0000_0000_0001, 0.75_f64.to_bits()),
        (0x000f_ffff_ffff_ffff, 0.75_f64.to_bits()),
        (0xbfe0_0000_0000_0000, 2049.0_f64.to_bits()),
        (0x3fff_ffff_ffff_ffff, 1024.0_f64.to_bits()),
        (0xbfe0_0000_0000_0000, 1073.0_f64.to_bits()),
        (0xbfe0_0000_0000_0000, 1074.0_f64.to_bits()),
    ];

    // Negative near-one bases keep finite results even for large integer exponents.
    // These exercise the word-split parity cases on JS and the huge-exponent reduction.
    let above_one = -(1.0_f64 + 2.0_f64.powi(-40));
    let below_one = -(1.0_f64 - 2.0_f64.powi(-40));
    let ulp_below_one = -(1.0_f64 - 2.0_f64.powi(-52));
    cases.extend([
        (above_one.to_bits(), 1_048_577.0_f64.to_bits()), // 2^20 + 1
        (below_one.to_bits(), 4_294_967_297.0_f64.to_bits()), // 2^32 + 1
        (below_one.to_bits(), 4_294_967_296.0_f64.to_bits()), // 2^32
        (above_one.to_bits(), 2_147_483_649.0_f64.to_bits()), // 2^31 + 1
        (
            ulp_below_one.to_bits(),
            9_007_199_254_740_994.0_f64.to_bits(),
        ), // 2^53 + 2
    ]);

    // Keep a deterministic bounded matrix of positive fractional powers and negative integer
    // powers. These values exercise varied exponents and significands while staying finite.
    for base_index in 0..24 {
        let base = 0.75 + base_index as f64 / 32.0;
        for exponent in [-16.0_f64, -2.5, -1.0, 0.5, 3.0, 16.0] {
            cases.push((base.to_bits(), exponent.to_bits()));
        }
    }
    for base_index in 0..12 {
        let base = -0.75 - base_index as f64 / 16.0;
        for exponent in [-15.0_f64, -4.0, 3.0, 12.0] {
            cases.push((base.to_bits(), exponent.to_bits()));
        }
    }

    let function_id = WasmLirFunctionId(0);
    let left = WasmLirLocalId(0);
    let right = WasmLirLocalId(1);
    let result_local = WasmLirLocalId(2);
    let mut module = WasmLirModule::default();
    module.functions.push(WasmLirFunction {
        id: function_id,
        debug_name: "checked_f64_power_parity".to_owned(),
        origin: WasmLirFunctionOrigin::ExportWrapper,
        signature: WasmLirSignature {
            params: vec![WasmAbiType::F64, WasmAbiType::F64],
            results: vec![WasmAbiType::F64],
        },
        locals: vec![
            WasmLirLocal {
                id: left,
                name: Some("base".to_owned()),
                ty: WasmAbiType::F64,
                role: WasmLocalRole::Param,
            },
            WasmLirLocal {
                id: right,
                name: Some("exponent".to_owned()),
                ty: WasmAbiType::F64,
                role: WasmLocalRole::Param,
            },
            local(result_local.0, WasmAbiType::F64, "power_result"),
        ],
        blocks: vec![WasmLirBlock {
            id: WasmLirBlockId(0),
            statements: vec![WasmLirStmt::CheckedFloatOp {
                dst: result_local,
                operator: NumericOperator::Power,
                precision: BinaryFloatPrecision::Binary64,
                operands: WasmNumericOperationOperands::Binary { left, right },
            }],
            terminator: WasmLirTerminator::Return {
                value: Some(result_local),
            },
        }],
        linkage: WasmFunctionLinkage::ExportedWrapper,
    });
    module.exports.push(WasmExport {
        export_name: "checked_f64_power".to_owned(),
        kind: WasmExportKind::Function(function_id),
    });

    let emitted = emit_lir_to_wasm_module(&module, &WasmBackendRequest::default())
        .expect("dynamic checked F64 power module should emit");
    validate_wasm(&emitted.wasm_bytes);

    let node_cases = cases
        .iter()
        .map(|(left_bits, right_bits)| format!("[\"{left_bits:016x}\", \"{right_bits:016x}\"]"))
        .collect::<Vec<_>>()
        .join(", ");
    const NODE_BODY: &str = r#"
const floatPower = new Function(__FLOAT_POWER_SOURCE__ + "\nreturn __moth_float_power;")();
const inputBits = new DataView(new ArrayBuffer(8));
const outputBits = new DataView(new ArrayBuffer(8));
const cases = [__POWER_CASES__];
const bitsOf = value => {
  if (!Number.isFinite(value)) {
    throw new Error("expected a finite power result");
  }
  outputBits.setFloat64(0, value, true);
  return outputBits.getBigUint64(0, true).toString(16).padStart(16, "0");
};
const results = cases.map(([leftBits, rightBits]) => {
  inputBits.setBigUint64(0, BigInt(`0x${leftBits}`), true);
  const left = inputBits.getFloat64(0, true);
  inputBits.setBigUint64(0, BigInt(`0x${rightBits}`), true);
  const right = inputBits.getFloat64(0, true);
  const wasmResult = instance.exports.checked_f64_power(left, right);
  const javascriptResult = floatPower(left, right);
  return `${bitsOf(wasmResult)}|${bitsOf(javascriptResult)}`;
});
process.stdout.write(results.join("\n"));
"#;
    let node_body = NODE_BODY
        .replace(
            "__FLOAT_POWER_SOURCE__",
            &format!("{:?}", include_str!("../../../js/runtime/float_power.js")),
        )
        .replace("__POWER_CASES__", &node_cases);
    let output = run_wasm_node_script(&emitted.wasm_bytes, "", &node_body);
    let output = String::from_utf8(output).expect("Node power parity output should be UTF-8");
    let results = output.lines().collect::<Vec<_>>();
    assert_eq!(results.len(), cases.len(), "Node should return every case");

    for (index, ((left_bits, right_bits), result_line)) in cases.iter().zip(results).enumerate() {
        let rust_result = crate::compiler_frontend::datatypes::numeric_power::pow(
            f64::from_bits(*left_bits),
            f64::from_bits(*right_bits),
        );
        assert!(
            rust_result.is_finite(),
            "case {index} (base={left_bits:016x}, exponent={right_bits:016x}) must be finite"
        );
        let expected_bits = format!("{:016x}", rust_result.to_bits());
        let (wasm_bits, javascript_bits) = result_line
            .split_once('|')
            .expect("Node should return Wasm and JavaScript result bits");
        assert_eq!(
            wasm_bits,
            expected_bits.as_str(),
            "Wasm mismatch in case {index} (base={left_bits:016x}, exponent={right_bits:016x})"
        );
        assert_eq!(
            javascript_bits,
            expected_bits.as_str(),
            "JavaScript mismatch in case {index} (base={left_bits:016x}, exponent={right_bits:016x})"
        );
    }
}

#[test]
fn scalar_storage_facts_follow_profile_and_natural_layout() {
    let kinds = [
        (WasmScalarStorageKind::I8, 1, WasmAbiType::I32),
        (WasmScalarStorageKind::U8, 1, WasmAbiType::I32),
        (WasmScalarStorageKind::I16, 2, WasmAbiType::I32),
        (WasmScalarStorageKind::U16, 2, WasmAbiType::I32),
        (WasmScalarStorageKind::F16, 2, WasmAbiType::F32),
        (WasmScalarStorageKind::I32, 4, WasmAbiType::I32),
        (WasmScalarStorageKind::U32, 4, WasmAbiType::I32),
        (WasmScalarStorageKind::I64, 8, WasmAbiType::I64),
        (WasmScalarStorageKind::U64, 8, WasmAbiType::I64),
        (WasmScalarStorageKind::F32, 4, WasmAbiType::F32),
        (WasmScalarStorageKind::F64, 8, WasmAbiType::F64),
    ];
    for (kind, size, carrier) in kinds {
        assert_eq!(kind.size(), size);
        assert_eq!(kind.alignment(), size);
        assert_eq!(kind.stride(), size);
        assert_eq!(kind.carrier(), carrier);
    }

    let profiles = [
        (
            IntWidth::Bits32,
            FloatPrecision::Bits32,
            WasmScalarStorageKind::I32,
            WasmScalarStorageKind::F32,
        ),
        (
            IntWidth::Bits32,
            FloatPrecision::Bits64,
            WasmScalarStorageKind::I32,
            WasmScalarStorageKind::F64,
        ),
        (
            IntWidth::Bits64,
            FloatPrecision::Bits32,
            WasmScalarStorageKind::I64,
            WasmScalarStorageKind::F32,
        ),
        (
            IntWidth::Bits64,
            FloatPrecision::Bits64,
            WasmScalarStorageKind::I64,
            WasmScalarStorageKind::F64,
        ),
    ];
    for (int_width, float_precision, int_kind, float_kind) in profiles {
        let profile = NumericProfile {
            int_width,
            float_precision,
        };
        assert_eq!(
            WasmScalarStorageKind::for_numeric_scalar(NumericScalar::Int, profile),
            Some(int_kind)
        );
        assert_eq!(
            WasmScalarStorageKind::for_numeric_scalar(NumericScalar::Float, profile),
            Some(float_kind)
        );
        assert_eq!(
            WasmScalarStorageKind::for_numeric_scalar(
                NumericScalar::Fixed(FixedScalar::F16),
                profile
            ),
            Some(WasmScalarStorageKind::F16)
        );
    }
    assert_eq!(
        WasmScalarStorageKind::from_fixed_scalar(FixedScalar::Byte),
        Some(WasmScalarStorageKind::U8)
    );
    assert_eq!(
        WasmScalarStorageKind::from_fixed_scalar(FixedScalar::F16),
        Some(WasmScalarStorageKind::F16)
    );
}

#[test]
fn binary16_helpers_are_planned_only_for_binary16_paths() {
    let mut memory_request = WasmBackendRequest::default();
    memory_request.export_policy.helper_exports.export_memory = true;
    let memory_plan = build_emit_plan(&WasmLirModule::default(), &memory_request)
        .expect("memory helper planning should succeed");
    assert!(
        !memory_plan
            .helper_indices
            .contains_key(&WasmRuntimeHelper::F32ToF16Bits)
    );
    assert!(
        !memory_plan
            .helper_indices
            .contains_key(&WasmRuntimeHelper::F16BitsToF32)
    );

    let round_plan = build_emit_plan(
        &build_round_f16_lir_module(),
        &WasmBackendRequest::default(),
    )
    .expect("binary16 helper planning should succeed");
    assert_eq!(
        helper_signature(WasmRuntimeHelper::F32ToF16Bits),
        WasmLirSignature {
            params: vec![WasmAbiType::F32],
            results: vec![WasmAbiType::I32],
        }
    );
    assert_eq!(
        helper_signature(WasmRuntimeHelper::F16BitsToF32),
        WasmLirSignature {
            params: vec![WasmAbiType::I32],
            results: vec![WasmAbiType::F32],
        }
    );
    assert_eq!(
        helper_name(WasmRuntimeHelper::F32ToF16Bits),
        "rt_f32_to_f16_bits"
    );
    assert_eq!(
        helper_name(WasmRuntimeHelper::F16BitsToF32),
        "rt_f16_bits_to_f32"
    );
    let helper_order = helper_emit_order();
    let to_half_index = helper_order
        .iter()
        .position(|helper| *helper == WasmRuntimeHelper::F32ToF16Bits)
        .expect("F32-to-binary16 helper should have a stable position");
    let from_half_index = helper_order
        .iter()
        .position(|helper| *helper == WasmRuntimeHelper::F16BitsToF32)
        .expect("binary16-to-F32 helper should have a stable position");
    assert!(to_half_index < from_half_index);
    let f32_to_f16 = round_plan
        .helper_indices
        .get(&WasmRuntimeHelper::F32ToF16Bits)
        .copied()
        .expect("rounding needs F32-to-binary16 conversion");
    let f16_to_f32 = round_plan
        .helper_indices
        .get(&WasmRuntimeHelper::F16BitsToF32)
        .copied()
        .expect("rounding needs binary16-to-F32 conversion");
    assert!(f32_to_f16 < f16_to_f32);
    assert!(round_plan.heap_top_global_index.is_none());
    assert!(
        !round_plan
            .helper_indices
            .contains_key(&WasmRuntimeHelper::Alloc)
    );
}

#[test]
fn binary16_rounding_matches_rust_for_f32_inputs_and_traps_invalid_values() {
    let module = build_round_f16_lir_module();
    let result =
        emit_lir_to_wasm_module(&module, &WasmBackendRequest::default()).expect("rounding LIR");
    validate_wasm(&result.wasm_bytes);

    let min_subnormal = f32::from_bits(0x3380_0000);
    let min_subnormal_tie = f32::from_bits(0x3300_0000);
    let normal_subnormal_tie = 2f32.powi(-14) - 2f32.powi(-25);
    let even_normal_tie = 1.0 + 2f32.powi(-11);
    let odd_normal_tie = 1.0 + 3.0 * 2f32.powi(-11);
    let overflow_tie = 65520.0f32;
    let mut inputs = vec![
        0.0,
        -0.0,
        min_subnormal_tie,
        -min_subnormal_tie,
        min_subnormal,
        3.0 * min_subnormal_tie,
        5.0 * min_subnormal_tie,
        normal_subnormal_tie,
        f32::from_bits(normal_subnormal_tie.to_bits() - 1),
        f32::from_bits(normal_subnormal_tie.to_bits() + 1),
        even_normal_tie,
        odd_normal_tie,
        f32::from_bits(even_normal_tie.to_bits() - 1),
        f32::from_bits(even_normal_tie.to_bits() + 1),
        65504.0,
        f32::from_bits(overflow_tie.to_bits() - 1),
        overflow_tie,
        f32::from_bits(overflow_tie.to_bits() + 1),
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
    ];
    let mut random_bits = 0x5a17_39cdu32;
    for _ in 0..128 {
        random_bits = random_bits
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        inputs.push(f32::from_bits(random_bits));
    }

    let input_bits = inputs
        .iter()
        .map(|value| format!("0x{:08x}", value.to_bits()))
        .collect::<Vec<_>>()
        .join(", ");
    let node_body = r#"
const inputs = [__INPUT_BITS__];
const view = new DataView(new ArrayBuffer(4));
const results = [];
for (let index = 0; index < inputs.length; index += 1) {
  view.setUint32(0, inputs[index], true);
  const value = view.getFloat32(0, true);
  try {
    const rounded = instance.exports.round(value);
    view.setFloat32(0, rounded, true);
    results.push(`${index}|ok|${view.getUint32(0, true).toString(16).padStart(8, "0")}`);
  } catch (error) {
    if (error !== null
        && typeof error === "object"
        && Object.getPrototypeOf(error) === WebAssembly.RuntimeError.prototype) {
      results.push(`${index}|trap`);
    } else {
      throw error;
    }
  }
}
const arithmetic = instance.exports.f32_add();
view.setFloat32(0, arithmetic, true);
results.push(`f32|${view.getUint32(0, true).toString(16).padStart(8, "0")}`);
process.stdout.write(results.join("\n"));
"#
    .replace("__INPUT_BITS__", &input_bits);
    let output = run_wasm_node_script(&result.wasm_bytes, "", &node_body);
    let actual = String::from_utf8(output)
        .expect("Node binary16 results should be UTF-8")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(actual.len(), inputs.len() + 1);

    for (index, (input, actual)) in inputs.iter().zip(&actual).enumerate() {
        let rounded = round_f64_to_f16(f64::from(*input));
        if rounded.is_finite() {
            assert_eq!(
                actual.as_str(),
                format!("{index}|ok|{:08x}", (rounded as f32).to_bits()).as_str(),
                "binary16 mismatch for F32 bits {:08x}",
                input.to_bits()
            );
        } else {
            assert_eq!(
                actual.as_str(),
                format!("{index}|trap").as_str(),
                "invalid F16 result did not trap for F32 bits {:08x}",
                input.to_bits()
            );
        }
    }
    let arithmetic_midpoint = 1.0f32 + 2f32.powi(-11);
    assert_eq!(
        actual[inputs.len()].as_str(),
        format!("f32|{:08x}", arithmetic_midpoint.to_bits()).as_str(),
        "F32 arithmetic stays unrounded until the explicit RoundF16 boundary"
    );
}

#[test]
fn scalar_storage_executes_in_node_with_compact_bytes_and_typed_loads() {
    let module = build_scalar_storage_lir_module();
    let mut request = WasmBackendRequest::default();
    request.export_policy.helper_exports.export_memory = true;

    let result = emit_lir_to_wasm_module(&module, &request).expect("scalar LIR should emit");
    validate_wasm(&result.wasm_bytes);
    let actual = execute_wasm_in_node(&result.wasm_bytes);

    let signed_i32 = -0x0123_4567i32;
    let unsigned_i32 = 0xfedc_ba98u32 as i32;
    let signed_i64 = -0x0123_4567_89ab_cdefi64;
    let unsigned_i64 = 0xfedc_ba98_7654_3210u64 as i64;
    let negative_zero_f32 = -0.0f32;
    let negative_zero_f64 = -0.0f64;
    let mut expected = vec![0u8; 152];
    expected[..64].fill(b'~');
    expected[1] = 0xfe;
    expected[2] = 0xfe;
    expected[4..6].copy_from_slice(&(-0x1234i16).to_le_bytes());
    expected[6..8].copy_from_slice(&0xfedcu16.to_le_bytes());
    expected[12..16].copy_from_slice(&signed_i32.to_le_bytes());
    expected[16..20].copy_from_slice(&0xfedc_ba98u32.to_le_bytes());
    expected[20..24].copy_from_slice(&negative_zero_f32.to_bits().to_le_bytes());
    expected[32..40].copy_from_slice(&signed_i64.to_le_bytes());
    expected[40..48].copy_from_slice(&0xfedc_ba98_7654_3210u64.to_le_bytes());
    expected[48..56].copy_from_slice(&negative_zero_f64.to_bits().to_le_bytes());
    expected[64..68].copy_from_slice(&(-2i32).to_le_bytes());
    expected[68..72].copy_from_slice(&254i32.to_le_bytes());
    expected[72..76].copy_from_slice(&(-0x1234i32).to_le_bytes());
    expected[76..80].copy_from_slice(&0xfedcu32.to_le_bytes());
    expected[80..84].copy_from_slice(&signed_i32.to_le_bytes());
    expected[84..88].copy_from_slice(&unsigned_i32.to_le_bytes());
    expected[88..96].copy_from_slice(&signed_i64.to_le_bytes());
    expected[96..104].copy_from_slice(&unsigned_i64.to_le_bytes());
    expected[104..108].copy_from_slice(&negative_zero_f32.to_bits().to_le_bytes());
    // Keep 108..112 untouched so the F64 round trip remains naturally aligned.
    expected[112..120].copy_from_slice(&negative_zero_f64.to_bits().to_le_bytes());

    expected[120..122].copy_from_slice(&0x8000u16.to_le_bytes());
    expected[122..124].copy_from_slice(&1u16.to_le_bytes());
    expected[124..126].copy_from_slice(&0x7bffu16.to_le_bytes());
    expected[126..128].copy_from_slice(&0xa55au16.to_le_bytes());
    expected[128..130].copy_from_slice(&0x8000u16.to_le_bytes());
    expected[130..132].copy_from_slice(&1u16.to_le_bytes());
    expected[132..134].copy_from_slice(&0x7bffu16.to_le_bytes());
    expected[134..136].copy_from_slice(&0xa55au16.to_le_bytes());
    expected[140..144].copy_from_slice(&negative_zero_f32.to_bits().to_le_bytes());
    expected[144..148].copy_from_slice(&(2f32.powi(-24)).to_bits().to_le_bytes());
    expected[148..152].copy_from_slice(&(65504.0f32).to_bits().to_le_bytes());

    assert_eq!(actual, expected);
}

#[test]
fn integer_text_helpers_return_exact_utf8_bytes_from_node_instantiated_wasm() {
    let module = build_integer_string_conversion_lir_module();
    let mut request = WasmBackendRequest::default();
    request.export_policy.helper_exports.export_memory = true;

    let result = emit_lir_to_wasm_module(&module, &request).expect("integer text LIR should emit");
    validate_wasm(&result.wasm_bytes);
    let sections = collect_section_order(&result.wasm_bytes);
    assert!(
        !sections.iter().any(|section| section == "import"),
        "integer string conversion must not add a host import"
    );

    let node_body = r#"
const cases = [
  ["signed_i32", -2147483648, "-2147483648"],
  ["unsigned_i32", -1, "4294967295"],
  ["signed_i64", -9223372036854775808n, "-9223372036854775808"],
  ["signed_i64", 9223372036854775807n, "9223372036854775807"],
  ["unsigned_i64", BigInt.asIntN(64, 18446744073709551615n), "18446744073709551615"],
  ["unsigned_i64", 0n, "0"],
];
const view = new DataView(instance.exports.memory.buffer);
const results = [];
for (const [name, input, expected] of cases) {
  const handle = instance.exports[name](input);
  const pointer = view.getUint32(handle, true);
  const length = view.getUint32(handle + 4, true);
  const actual = new TextDecoder().decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length));
  if (actual !== expected) {
    throw new Error(`${name}: expected ${expected}, got ${actual}`);
  }
  results.push(`${name}|${actual}`);
}
process.stdout.write(results.join("\n"));
"#;
    let output = run_wasm_node_script(&result.wasm_bytes, "", node_body);
    let actual = String::from_utf8(output).expect("Node integer string results should be UTF-8");
    assert_eq!(
        actual,
        "signed_i32|-2147483648\nunsigned_i32|4294967295\nsigned_i64|-9223372036854775808\nsigned_i64|9223372036854775807\nunsigned_i64|18446744073709551615\nunsigned_i64|0"
    );
}

#[test]
fn float_text_helpers_return_canonical_utf8_from_node_instantiated_wasm() {
    let module = build_float_string_conversion_lir_module();
    let mut request = WasmBackendRequest::default();
    request.export_policy.helper_exports.export_memory = true;

    let result = emit_lir_to_wasm_module(&module, &request).expect("float text LIR should emit");
    validate_wasm(&result.wasm_bytes);
    let sections = collect_section_order(&result.wasm_bytes);
    assert!(
        !sections.iter().any(|section| section == "import"),
        "float string conversion must not add a host import"
    );

    let f32_bits = float_text_f32_cases();
    let f64_bits = float_text_f64_cases();
    let f32_js_bits = f32_bits
        .iter()
        .map(|bits| format!("0x{bits:08x}"))
        .collect::<Vec<_>>()
        .join(", ");
    let f64_js_bits = f64_bits
        .iter()
        .map(|bits| format!("0x{bits:016x}n"))
        .collect::<Vec<_>>()
        .join(", ");
    let node_body = r#"
const f32Bits = [__F32_BITS__];
const f64Bits = [__F64_BITS__];
const rows = [];
const utf8 = new TextDecoder("utf-8", { fatal: true });
function readString(handle) {
  // Runtime formatting may grow memory, so never retain a buffer from before the call.
  const memory = instance.exports.memory.buffer;
  const view = new DataView(memory);
  const pointer = view.getUint32(handle, true);
  const length = view.getUint32(handle + 4, true);
  return utf8.decode(new Uint8Array(memory, pointer, length));
}
function f16BitsToF32(bits) {
  const sign = (bits & 0x8000) === 0 ? 1 : -1;
  const exponent = (bits >>> 10) & 0x1f;
  const fraction = bits & 0x03ff;
  if (exponent === 0) {
    return sign * fraction * (2 ** -24);
  }
  return sign * (1 + fraction / 1024) * (2 ** (exponent - 15));
}
for (const sign of [0, 0x8000]) {
  for (let magnitude = 0; magnitude < 0x7c00; magnitude += 1) {
    const bits = sign | magnitude;
    const value = f16BitsToF32(bits);
    let handle;
    try {
      handle = instance.exports.string_f16(value);
    } catch (error) {
      throw new Error(`F16 bits 0x${bits.toString(16).padStart(4, "0")}: ${error.stack}`);
    }
    rows.push(`f16|${bits.toString(16).padStart(4, "0")}|${readString(handle)}`);
  }
}
const f32View = new DataView(new ArrayBuffer(4));
for (const bits of f32Bits) {
  f32View.setUint32(0, bits, true);
  const value = f32View.getFloat32(0, true);
  const handle = instance.exports.string_f32(value);
  rows.push(`f32|${bits.toString(16).padStart(8, "0")}|${readString(handle)}`);
}
const f64View = new DataView(new ArrayBuffer(8));
for (const bits of f64Bits) {
  f64View.setBigUint64(0, bits, true);
  const value = f64View.getFloat64(0, true);
  const handle = instance.exports.string_f64(value);
  rows.push(`f64|${bits.toString(16).padStart(16, "0")}|${readString(handle)}`);
}

// The same F32 carrier value is formatted once as F16 and once as F32.
const halfPrecisionProbe = 65504;
rows.push(`f16-probe|7bff|${readString(instance.exports.string_f16(halfPrecisionProbe))}`);
rows.push(`f32-probe|477fe000|${readString(instance.exports.string_f32(halfPrecisionProbe))}`);

// The F64 call receives the exact promotion of this same F32 value.
f32View.setUint32(0, 0x3dcccccd, true);
const promotedF32 = f32View.getFloat32(0, true);
f64View.setFloat64(0, promotedF32, true);
const promotedF64Bits = f64View.getBigUint64(0, true);
rows.push(`f32-promoted-probe|3dcccccd|${readString(instance.exports.string_f32(promotedF32))}`);
rows.push(`f64-promoted-probe|${promotedF64Bits.toString(16).padStart(16, "0")}|${readString(instance.exports.string_f64(promotedF32))}`);

function expectNonFiniteTrap(precision, bits, call) {
  try {
    call();
  } catch (error) {
    if (error instanceof WebAssembly.RuntimeError
        && error.message.toLowerCase().includes("unreachable")) {
      rows.push(`trap|${precision}|${bits}|ok`);
      return;
    }
    throw error;
  }
  throw new Error(`${precision} formatter accepted non-finite bits ${bits}`);
}
for (const [bits, label] of [
  [0x7fc00000, "7fc00000"],
  [0x7f800000, "7f800000"],
  [0xff800000, "ff800000"],
]) {
  f32View.setUint32(0, bits, true);
  const value = f32View.getFloat32(0, true);
  expectNonFiniteTrap("f16", label, () => instance.exports.string_f16(value));
  expectNonFiniteTrap("f32", label, () => instance.exports.string_f32(value));
}
for (const [bits, label] of [
  [0x7ff8000000000000n, "7ff8000000000000"],
  [0x7ff0000000000000n, "7ff0000000000000"],
  [0xfff0000000000000n, "fff0000000000000"],
]) {
  f64View.setBigUint64(0, bits, true);
  const value = f64View.getFloat64(0, true);
  expectNonFiniteTrap("f64", label, () => instance.exports.string_f64(value));
}
process.stdout.write(rows.join("\n"));
"#
    .replace("__F32_BITS__", &f32_js_bits)
    .replace("__F64_BITS__", &f64_js_bits);
    let output = run_wasm_node_script(&result.wasm_bytes, "", &node_body);
    let actual = String::from_utf8(output)
        .expect("Node float string results should be UTF-8")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();

    let mut expected = Vec::with_capacity(63_488 + f32_bits.len() + f64_bits.len() + 17);
    for sign in [0u16, 0x8000] {
        for magnitude in 0..0x7c00 {
            let bits = sign | magnitude;
            push_float_text_expectation(
                &mut expected,
                "f16",
                format!("{bits:04x}"),
                binary16_bits_as_f64(bits),
                BinaryFloatPrecision::Binary16,
            );
        }
    }
    for bits in &f32_bits {
        push_float_text_expectation(
            &mut expected,
            "f32",
            format!("{bits:08x}"),
            f64::from(f32::from_bits(*bits)),
            BinaryFloatPrecision::Binary32,
        );
    }
    for bits in &f64_bits {
        push_float_text_expectation(
            &mut expected,
            "f64",
            format!("{bits:016x}"),
            f64::from_bits(*bits),
            BinaryFloatPrecision::Binary64,
        );
    }

    let half_probe = 65504.0f32;
    let half_probe_value = f64::from(half_probe);
    let half_text = format_finite_float(half_probe_value, BinaryFloatPrecision::Binary16)
        .expect("finite F16 probe should format");
    let f32_text = format_finite_float(half_probe_value, BinaryFloatPrecision::Binary32)
        .expect("finite F32 probe should format");
    assert_ne!(
        half_text, f32_text,
        "the F16/F32 probe must distinguish precision for F32 bits 477fe000"
    );
    expected.push(format!("f16-probe|7bff|{half_text}"));
    expected.push(format!("f32-probe|477fe000|{f32_text}"));

    let promoted_f32 = f32::from_bits(0x3dcccccd);
    let promoted_f64 = f64::from(promoted_f32);
    let promoted_f32_text = format_finite_float(promoted_f64, BinaryFloatPrecision::Binary32)
        .expect("finite promoted F32 probe should format");
    let promoted_f64_text = format_finite_float(promoted_f64, BinaryFloatPrecision::Binary64)
        .expect("finite promoted F64 probe should format");
    assert_ne!(
        promoted_f32_text, promoted_f64_text,
        "the F32/F64 probe must distinguish precision for promoted F32 bits 3dcccccd"
    );
    expected.push(format!("f32-promoted-probe|3dcccccd|{promoted_f32_text}"));
    expected.push(format!(
        "f64-promoted-probe|{:016x}|{promoted_f64_text}",
        promoted_f64.to_bits()
    ));
    for (precision, bits) in [
        ("f16", "7fc00000"),
        ("f32", "7fc00000"),
        ("f16", "7f800000"),
        ("f32", "7f800000"),
        ("f16", "ff800000"),
        ("f32", "ff800000"),
        ("f64", "7ff8000000000000"),
        ("f64", "7ff0000000000000"),
        ("f64", "fff0000000000000"),
    ] {
        expected.push(format!("trap|{precision}|{bits}|ok"));
    }

    let next_expected_row = expected.get(actual.len()).or_else(|| expected.last());
    assert_eq!(
        actual.len(),
        expected.len(),
        "Node float output row count mismatch near {next_expected_row:?}"
    );
    for (expected_row, actual_row) in expected.iter().zip(&actual) {
        assert_eq!(
            actual_row, expected_row,
            "Wasm float formatter mismatch for {expected_row}"
        );
    }
}

#[test]
fn binary16_scalar_storage_rounds_directly_and_traps_nonfinite_values() {
    let module = build_binary16_scalar_storage_boundary_module();
    let mut request = WasmBackendRequest::default();
    request.export_policy.helper_exports.export_memory = true;

    let result = emit_lir_to_wasm_module(&module, &request).expect("binary16 storage LIR");
    validate_wasm(&result.wasm_bytes);
    let node_body = r#"
const assert = require("node:assert/strict");
const view = new DataView(instance.exports.memory.buffer);
const isExactRuntimeError = error => error !== null
    && typeof error === "object"
    && Object.getPrototypeOf(error) === WebAssembly.RuntimeError.prototype;

view.setUint8(2, 0xa5);
const input = new DataView(new ArrayBuffer(4));
input.setUint32(0, 0x3f801001, true);
instance.exports.store_f16(input.getFloat32(0, true));
assert.equal(view.getUint16(0, true), 0x3c01,
    "F16 store must round the non-canonical F32 directly to 0x3c01");
assert.equal(view.getUint8(2), 0xa5,
    "F16 store must leave the following byte untouched");
const loaded = instance.exports.load_f16();
const loadedBits = new DataView(new ArrayBuffer(4));
loadedBits.setFloat32(0, loaded, true);
assert.equal(loadedBits.getUint32(0, true), 0x3f802000,
    "normal F16 load must return the rounded finite value");

view.setUint16(0, 0x3555, true);
assert.throws(() => instance.exports.store_f16(65520.0), isExactRuntimeError,
    "F16 overflow store must trap with WebAssembly.RuntimeError");
const afterOverflow = view.getUint16(0, true);
assert.equal(afterOverflow, 0x3555,
    "trapping F16 store must not publish an overflow result");

for (const invalidBits of [0x7c00, 0xfe00]) {
  view.setUint16(0, invalidBits, true);
  assert.throws(() => instance.exports.load_f16(), isExactRuntimeError,
      `F16 load of 0x${invalidBits.toString(16)} must trap with WebAssembly.RuntimeError`);
}
"#;
    run_wasm_node_script(&result.wasm_bytes, "", node_body);
}

fn build_binary16_scalar_storage_boundary_module() -> WasmLirModule {
    let f16_kind = WasmScalarStorageKind::from_fixed_scalar(FixedScalar::F16)
        .expect("F16 has fixed scalar storage");

    let source = WasmLirLocalId(0);
    let store_address = WasmLirLocalId(1);
    let store_function = WasmLirFunction {
        id: WasmLirFunctionId(0),
        debug_name: "store_f16".to_owned(),
        origin: WasmLirFunctionOrigin::ExportWrapper,
        signature: WasmLirSignature {
            params: vec![WasmAbiType::F32],
            results: vec![],
        },
        locals: vec![
            WasmLirLocal {
                id: source,
                name: Some("source".to_owned()),
                ty: WasmAbiType::F32,
                role: WasmLocalRole::Param,
            },
            local(store_address.0, WasmAbiType::I32, "address"),
        ],
        blocks: vec![WasmLirBlock {
            id: WasmLirBlockId(0),
            statements: vec![
                WasmLirStmt::ConstI32 {
                    dst: store_address,
                    value: 0,
                },
                WasmLirStmt::StoreScalar {
                    address: store_address,
                    offset: 0,
                    value: source,
                    kind: f16_kind,
                },
            ],
            terminator: WasmLirTerminator::Return { value: None },
        }],
        linkage: WasmFunctionLinkage::ExportedWrapper,
    };

    let load_address = WasmLirLocalId(0);
    let loaded = WasmLirLocalId(1);
    let load_function = WasmLirFunction {
        id: WasmLirFunctionId(1),
        debug_name: "load_f16".to_owned(),
        origin: WasmLirFunctionOrigin::ExportWrapper,
        signature: WasmLirSignature {
            params: vec![],
            results: vec![WasmAbiType::F32],
        },
        locals: vec![
            local(load_address.0, WasmAbiType::I32, "address"),
            local(loaded.0, WasmAbiType::F32, "loaded"),
        ],
        blocks: vec![WasmLirBlock {
            id: WasmLirBlockId(0),
            statements: vec![
                WasmLirStmt::ConstI32 {
                    dst: load_address,
                    value: 0,
                },
                WasmLirStmt::LoadScalar {
                    dst: loaded,
                    address: load_address,
                    offset: 0,
                    kind: f16_kind,
                },
            ],
            terminator: WasmLirTerminator::Return {
                value: Some(loaded),
            },
        }],
        linkage: WasmFunctionLinkage::ExportedWrapper,
    };

    WasmLirModule {
        functions: vec![store_function, load_function],
        imports: vec![],
        exports: vec![
            WasmExport {
                export_name: "store_f16".to_owned(),
                kind: WasmExportKind::Function(WasmLirFunctionId(0)),
            },
            WasmExport {
                export_name: "load_f16".to_owned(),
                kind: WasmExportKind::Function(WasmLirFunctionId(1)),
            },
        ],
        static_data: vec![],
        memory_plan: WasmMemoryPlan::default(),
    }
}

#[test]
fn scalar_storage_rejects_mismatched_local_carrier() {
    let mut module = build_scalar_storage_lir_module();
    module.functions[0].locals[1].ty = WasmAbiType::I64;

    let error = emit_lir_to_wasm_module(&module, &WasmBackendRequest::default())
        .expect_err("a scalar memory store with the wrong carrier must fail emission");
    assert_eq!(
        error.error_type,
        ErrorType::Backend(BackendErrorType::WasmGeneration)
    );
    assert!(error.msg.contains("scalar store value"));
}

#[test]
fn rejects_unsupported_wasm_feature_flags() {
    let mut request = WasmBackendRequest::default();
    request.target_features.use_wasm_gc = true;

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let start_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");
    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(1, 0, types.int, RegionId(0))),
    };
    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.int,
    };
    let hir_module = build_module(
        &mut path_fork,
        &mut string_table,
        vec![(start_function, start_path, HirFunctionOrigin::EntryStart)],
        vec![start_block],
        FunctionId(0),
    );

    let error = lower_hir_to_wasm_module(
        &hir_module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &request,
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect_err("unsupported feature toggle should fail");
    let error = error
        .infrastructure_error()
        .expect("Wasm generation failure should be wrapped for rendering");
    assert_eq!(
        &error.error_type,
        &ErrorType::Backend(BackendErrorType::WasmGeneration)
    );
    assert!(error.msg.contains("use_wasm_gc"));
}

#[test]
fn rejects_unsupported_cfg_lowering_strategy() {
    let mut request = WasmBackendRequest::default();
    request.emit_options.cfg_lowering_strategy = WasmCfgLoweringStrategy::Structured;

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let start_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");
    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(1, 0, types.int, RegionId(0))),
    };
    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.int,
    };
    let hir_module = build_module(
        &mut path_fork,
        &mut string_table,
        vec![(start_function, start_path, HirFunctionOrigin::EntryStart)],
        vec![start_block],
        FunctionId(0),
    );

    let error = lower_hir_to_wasm_module(
        &hir_module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &request,
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect_err("unsupported cfg strategy should fail");
    let error = error
        .infrastructure_error()
        .expect("Wasm generation failure should be wrapped for rendering");
    assert_eq!(
        &error.error_type,
        &ErrorType::Backend(BackendErrorType::WasmGeneration)
    );
    assert!(error.msg.contains("dispatcher-loop"));
}

#[test]
fn debug_output_reports_aligned_static_offsets() {
    let mut module = build_manual_lir_module();
    module.static_data.push(WasmStaticData {
        id: WasmStaticDataId(1),
        debug_name: "manual.literal.b".to_owned(),
        bytes: b"abc".to_vec(),
        kind: WasmStaticDataKind::Utf8StringBytes,
    });

    let mut request = WasmBackendRequest::default();
    request.emit_options.validate_emitted_module = false;
    let emit_result = emit_lir_to_wasm_module(&module, &request).expect("emit should succeed");
    let text = emit_result.debug_outputs.data_layout_text;

    let mut offsets = Vec::new();
    for line in text.lines() {
        if let Some(position) = line.find("offset=") {
            let rest = &line[position + "offset=".len()..];
            let digits = rest
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>();
            if let Ok(offset) = digits.parse::<u32>() {
                offsets.push(offset);
            }
        }
    }

    assert!(!offsets.is_empty());
    assert!(offsets.into_iter().all(|offset| offset % 8 == 0));
}

fn validate_wasm(bytes: &[u8]) {
    wasmparser::Validator::new()
        .validate_all(bytes)
        .expect("emitted bytes should validate");
}

fn collect_export_names(bytes: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(bytes) {
        let payload = payload.expect("payload should parse");
        if let wasmparser::Payload::ExportSection(reader) = payload {
            for export in reader {
                let export = export.expect("export should parse");
                names.push(export.name.to_owned());
            }
        }
    }
    names
}

fn collect_section_order(bytes: &[u8]) -> Vec<String> {
    let mut order = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(bytes) {
        let payload = payload.expect("payload should parse");
        match payload {
            wasmparser::Payload::TypeSection(_) => order.push("type".to_owned()),
            wasmparser::Payload::ImportSection(_) => order.push("import".to_owned()),
            wasmparser::Payload::FunctionSection(_) => order.push("function".to_owned()),
            wasmparser::Payload::MemorySection(_) => order.push("memory".to_owned()),
            wasmparser::Payload::GlobalSection(_) => order.push("global".to_owned()),
            wasmparser::Payload::ExportSection(_) => order.push("export".to_owned()),
            wasmparser::Payload::CodeSectionStart { .. } => order.push("code".to_owned()),
            wasmparser::Payload::DataSection(_) => order.push("data".to_owned()),
            _ => {}
        }
    }
    order
}

fn assert_order(order: &[String], first: &str, second: &str) {
    let first_index = order
        .iter()
        .position(|section| section == first)
        .expect("first section should exist");
    let second_index = order
        .iter()
        .position(|section| section == second)
        .expect("second section should exist");
    assert!(
        first_index < second_index,
        "{first} should appear before {second}"
    );
}

fn request_with_helper_exports() -> WasmBackendRequest {
    WasmBackendRequest {
        export_policy: WasmExportPolicy {
            exported_functions: vec![],
            export_names: FxHashMap::default(),
            helper_exports: WasmHelperExportPolicy {
                export_memory: true,
                export_str_ptr: true,
                export_str_len: true,
                export_vec_new: true,
                export_vec_push: true,
                export_vec_len: true,
                export_vec_get: true,
                export_release: true,
            },
        },
        target_features: WasmTargetFeatures::default(),
        emit_options: WasmEmitOptions {
            emit_wasm_module: true,
            validate_emitted_module: false,
            emit_name_section: false,
            cfg_lowering_strategy: WasmCfgLoweringStrategy::DispatcherLoop,
        },
        debug_flags: WasmDebugFlags {
            show_wasm_data_layout: true,
            show_wasm_indices: true,
            show_wasm_sections: true,
            show_wasm_validation: true,
            ..Default::default()
        },
        external_package_registry: Default::default(),
        structural_string_urls: None,
        function_emission_policy: Default::default(),
        numeric_profile: NumericProfile::STANDARD,
    }
}

fn build_manual_lir_module() -> WasmLirModule {
    let callee = WasmLirFunction {
        id: WasmLirFunctionId(0),
        debug_name: "manual_callee".to_owned(),
        origin: WasmLirFunctionOrigin::Normal,
        signature: WasmLirSignature {
            params: vec![],
            results: vec![WasmAbiType::I32],
        },
        locals: vec![WasmLirLocal {
            id: WasmLirLocalId(0),
            name: Some("ret".to_owned()),
            ty: WasmAbiType::I32,
            role: WasmLocalRole::Temp,
        }],
        blocks: vec![WasmLirBlock {
            id: WasmLirBlockId(0),
            statements: vec![WasmLirStmt::ConstI32 {
                dst: WasmLirLocalId(0),
                value: 7,
            }],
            terminator: WasmLirTerminator::Return {
                value: Some(WasmLirLocalId(0)),
            },
        }],
        linkage: WasmFunctionLinkage::Internal,
    };

    let main = WasmLirFunction {
        id: WasmLirFunctionId(1),
        debug_name: "manual_main".to_owned(),
        origin: WasmLirFunctionOrigin::EntryStart,
        signature: WasmLirSignature {
            params: vec![],
            results: vec![],
        },
        locals: vec![
            local(0, WasmAbiType::I32, "i32"),
            local(1, WasmAbiType::I64, "i64"),
            local(2, WasmAbiType::F32, "f32"),
            local(3, WasmAbiType::F64, "f64"),
            local(4, WasmAbiType::Handle, "buffer_a"),
            local(5, WasmAbiType::Handle, "string_a"),
            local(6, WasmAbiType::I32, "static_ptr"),
            local(7, WasmAbiType::I32, "len"),
            local(8, WasmAbiType::Handle, "buffer_b"),
            local(9, WasmAbiType::Handle, "string_b"),
            local(10, WasmAbiType::I32, "eq"),
            local(11, WasmAbiType::I32, "ne"),
            local(12, WasmAbiType::I32, "call_result"),
            local(13, WasmAbiType::I64, "sum_i64"),
            local(14, WasmAbiType::F64, "sum_f64"),
            local(15, WasmAbiType::I32, "lt_i64"),
            local(16, WasmAbiType::I32, "le_i64"),
            local(17, WasmAbiType::I32, "gt_i64"),
            local(18, WasmAbiType::I32, "ge_i64"),
            local(19, WasmAbiType::I32, "lt_f64"),
            local(20, WasmAbiType::I64, "sub_i64"),
            local(21, WasmAbiType::F64, "sub_f64"),
            local(22, WasmAbiType::Handle, "string_from_i64"),
        ],
        blocks: vec![WasmLirBlock {
            id: WasmLirBlockId(0),
            statements: vec![
                WasmLirStmt::ConstI32 {
                    dst: WasmLirLocalId(0),
                    value: 1,
                },
                WasmLirStmt::ConstI64 {
                    dst: WasmLirLocalId(1),
                    value: 2,
                },
                WasmLirStmt::ConstF32 {
                    dst: WasmLirLocalId(2),
                    value: 3.5,
                },
                WasmLirStmt::ConstF64 {
                    dst: WasmLirLocalId(3),
                    value: 4.5,
                },
                WasmLirStmt::ConstStaticPtr {
                    dst: WasmLirLocalId(6),
                    data: WasmStaticDataId(0),
                },
                WasmLirStmt::ConstLength {
                    dst: WasmLirLocalId(7),
                    value: 1,
                },
                WasmLirStmt::Copy {
                    dst: WasmLirLocalId(10),
                    src: WasmLirLocalId(0),
                },
                WasmLirStmt::Move {
                    dst: WasmLirLocalId(11),
                    src: WasmLirLocalId(10),
                },
                WasmLirStmt::Call {
                    dst: Some(WasmLirLocalId(12)),
                    callee: WasmCalleeRef::Function(WasmLirFunctionId(0)),
                    args: vec![],
                },
                WasmLirStmt::StringNewBuffer {
                    dst: WasmLirLocalId(4),
                },
                WasmLirStmt::StringPushLiteral {
                    buffer: WasmLirLocalId(4),
                    data: WasmStaticDataId(0),
                },
                WasmLirStmt::StringFinish {
                    dst: WasmLirLocalId(5),
                    buffer: WasmLirLocalId(4),
                },
                WasmLirStmt::StringNewBuffer {
                    dst: WasmLirLocalId(8),
                },
                WasmLirStmt::StringPushHandle {
                    buffer: WasmLirLocalId(8),
                    handle: WasmLirLocalId(5),
                },
                WasmLirStmt::StringFromI64 {
                    dst: WasmLirLocalId(22),
                    value: WasmLirLocalId(1),
                },
                WasmLirStmt::StringPushHandle {
                    buffer: WasmLirLocalId(8),
                    handle: WasmLirLocalId(22),
                },
                WasmLirStmt::StringFinish {
                    dst: WasmLirLocalId(9),
                    buffer: WasmLirLocalId(8),
                },
                WasmLirStmt::Call {
                    dst: None,
                    callee: WasmCalleeRef::Import(WasmImportId(0)),
                    args: vec![WasmLirLocalId(9)],
                },
                WasmLirStmt::DropIfOwned {
                    value: WasmLirLocalId(9),
                },
                WasmLirStmt::StringEq {
                    dst: WasmLirLocalId(10),
                    lhs: WasmLirLocalId(5),
                    rhs: WasmLirLocalId(9),
                },
                WasmLirStmt::StringNe {
                    dst: WasmLirLocalId(11),
                    lhs: WasmLirLocalId(5),
                    rhs: WasmLirLocalId(9),
                },
                WasmLirStmt::IntEq {
                    dst: WasmLirLocalId(10),
                    lhs: WasmLirLocalId(0),
                    rhs: WasmLirLocalId(0),
                },
                WasmLirStmt::IntNe {
                    dst: WasmLirLocalId(11),
                    lhs: WasmLirLocalId(0),
                    rhs: WasmLirLocalId(10),
                },
                WasmLirStmt::CheckedIntegerOp {
                    operation: WasmCheckedIntegerOperation {
                        dst: WasmLirLocalId(13),
                        operator: NumericOperator::Add,
                        kind: WasmIntegerOperationKind::Signed64,
                        check_mode: IntegerCheckMode::Runtime,
                        operands: WasmNumericOperationOperands::Binary {
                            left: WasmLirLocalId(1),
                            right: WasmLirLocalId(1),
                        },
                        scratch: WasmIntegerScratch {
                            product: None,
                            power: None,
                        },
                    },
                },
                WasmLirStmt::CheckedIntegerOp {
                    operation: WasmCheckedIntegerOperation {
                        dst: WasmLirLocalId(20),
                        operator: NumericOperator::Subtract,
                        kind: WasmIntegerOperationKind::Signed64,
                        check_mode: IntegerCheckMode::Runtime,
                        operands: WasmNumericOperationOperands::Binary {
                            left: WasmLirLocalId(13),
                            right: WasmLirLocalId(1),
                        },
                        scratch: WasmIntegerScratch {
                            product: None,
                            power: None,
                        },
                    },
                },
                WasmLirStmt::OrderedLt {
                    dst: WasmLirLocalId(15),
                    lhs: WasmLirLocalId(1),
                    rhs: WasmLirLocalId(13),
                },
                WasmLirStmt::OrderedLe {
                    dst: WasmLirLocalId(16),
                    lhs: WasmLirLocalId(1),
                    rhs: WasmLirLocalId(13),
                },
                WasmLirStmt::OrderedGt {
                    dst: WasmLirLocalId(17),
                    lhs: WasmLirLocalId(13),
                    rhs: WasmLirLocalId(1),
                },
                WasmLirStmt::OrderedGe {
                    dst: WasmLirLocalId(18),
                    lhs: WasmLirLocalId(13),
                    rhs: WasmLirLocalId(1),
                },
                WasmLirStmt::OrderedLt {
                    dst: WasmLirLocalId(19),
                    lhs: WasmLirLocalId(3),
                    rhs: WasmLirLocalId(14),
                },
            ],
            terminator: WasmLirTerminator::Return { value: None },
        }],
        linkage: WasmFunctionLinkage::ExportedWrapper,
    };

    WasmLirModule {
        functions: vec![callee, main],
        imports: vec![WasmImport {
            id: WasmImportId(0),
            module_name: "host".to_owned(),
            item_name: "test_import".to_owned(),
            kind: WasmImportKind::Function(WasmLirSignature {
                params: vec![WasmAbiType::Handle],
                results: vec![],
            }),
        }],
        exports: vec![WasmExport {
            export_name: "moth_call_0".to_owned(),
            kind: WasmExportKind::Function(WasmLirFunctionId(1)),
        }],
        static_data: vec![WasmStaticData {
            id: WasmStaticDataId(0),
            debug_name: "manual.literal.a".to_owned(),
            bytes: b"x".to_vec(),
            kind: WasmStaticDataKind::Utf8StringBytes,
        }],
        memory_plan: WasmMemoryPlan::default(),
    }
}

fn local(id: u32, ty: WasmAbiType, name: &str) -> WasmLirLocal {
    WasmLirLocal {
        id: WasmLirLocalId(id),
        name: Some(name.to_owned()),
        ty,
        role: WasmLocalRole::Temp,
    }
}

fn build_checked_integer_boundary_module() -> WasmLirModule {
    let cases = [
        (
            "u64_mul_zero",
            WasmIntegerOperationKind::Unsigned64,
            NumericOperator::Multiply,
            0,
            -1,
        ),
        (
            "u64_mul_overflow",
            WasmIntegerOperationKind::Unsigned64,
            NumericOperator::Multiply,
            -1,
            2,
        ),
        (
            "i32_mul_overflow_high",
            WasmIntegerOperationKind::Signed32,
            NumericOperator::Multiply,
            i32::MAX as i64,
            2,
        ),
        (
            "i32_mul_overflow_low",
            WasmIntegerOperationKind::Signed32,
            NumericOperator::Multiply,
            i32::MIN as i64,
            2,
        ),
        (
            "i64_mul_underflow_min_times_two",
            WasmIntegerOperationKind::Signed64,
            NumericOperator::Multiply,
            i64::MIN,
            2,
        ),
        (
            "i64_mul_underflow_two_times_min",
            WasmIntegerOperationKind::Signed64,
            NumericOperator::Multiply,
            2,
            i64::MIN,
        ),
        (
            "i32_div_min_by_negative_one",
            WasmIntegerOperationKind::Signed32,
            NumericOperator::IntegerDivide,
            i32::MIN as i64,
            -1,
        ),
        (
            "i64_add_overflow_high",
            WasmIntegerOperationKind::Signed64,
            NumericOperator::Add,
            i64::MAX,
            1,
        ),
        (
            "i64_add_overflow_low",
            WasmIntegerOperationKind::Signed64,
            NumericOperator::Add,
            i64::MIN,
            -1,
        ),
        (
            "i64_sub_overflow_low",
            WasmIntegerOperationKind::Signed64,
            NumericOperator::Subtract,
            i64::MIN,
            1,
        ),
        (
            "i64_sub_overflow_high",
            WasmIntegerOperationKind::Signed64,
            NumericOperator::Subtract,
            i64::MAX,
            -1,
        ),
        (
            "i64_power_preserves_sources",
            WasmIntegerOperationKind::Signed64,
            NumericOperator::Power,
            2,
            5,
        ),
    ];
    let mut module = WasmLirModule::default();
    for (function_index, (name, kind, operator, left_value, right_value)) in
        cases.into_iter().enumerate()
    {
        let function_id = WasmLirFunctionId(function_index as u32);
        let (function, export) = checked_integer_boundary_function(
            function_id,
            name,
            kind,
            operator,
            left_value,
            right_value,
        );
        module.functions.push(function);
        module.exports.push(export);
    }
    module
}

fn checked_integer_boundary_function(
    function_id: WasmLirFunctionId,
    name: &str,
    kind: WasmIntegerOperationKind,
    operator: NumericOperator,
    left_value: i64,
    right_value: i64,
) -> (WasmLirFunction, WasmExport) {
    let carrier = kind.carrier();
    let left = WasmLirLocalId(0);
    let right = WasmLirLocalId(1);
    let operation_result = WasmLirLocalId(2);
    let mut locals = vec![
        local(left.0, carrier, "left"),
        local(right.0, carrier, "right"),
        local(operation_result.0, carrier, "operation_result"),
    ];
    let mut statements = match carrier {
        WasmAbiType::I32 => vec![
            WasmLirStmt::ConstI32 {
                dst: left,
                value: left_value as i32,
            },
            WasmLirStmt::ConstI32 {
                dst: right,
                value: right_value as i32,
            },
        ],
        WasmAbiType::I64 => vec![
            WasmLirStmt::ConstI64 {
                dst: left,
                value: left_value,
            },
            WasmLirStmt::ConstI64 {
                dst: right,
                value: right_value,
            },
        ],
        _ => unreachable!("checked integer boundary cases use integer carriers"),
    };
    let (power_scratch, source_sum) = if operator == NumericOperator::Power {
        let factor = WasmLirLocalId(locals.len() as u32);
        locals.push(local(factor.0, carrier, "power_factor"));
        let exponent = WasmLirLocalId(locals.len() as u32);
        locals.push(local(exponent.0, carrier, "power_exponent"));
        let sum = WasmLirLocalId(locals.len() as u32);
        locals.push(local(sum.0, carrier, "source_sum"));
        (
            Some(WasmIntegerPowerScratch { factor, exponent }),
            Some(sum),
        )
    } else {
        (None, None)
    };
    let needs_product_scratch =
        matches!(
            kind,
            WasmIntegerOperationKind::Signed32 | WasmIntegerOperationKind::Unsigned32
        ) && matches!(operator, NumericOperator::Multiply | NumericOperator::Power);
    let product_scratch = if needs_product_scratch {
        let scratch = WasmLirLocalId(locals.len() as u32);
        locals.push(local(scratch.0, WasmAbiType::I64, "product_scratch"));
        Some(scratch)
    } else {
        None
    };
    statements.push(WasmLirStmt::CheckedIntegerOp {
        operation: WasmCheckedIntegerOperation {
            dst: operation_result,
            operator,
            kind,
            check_mode: IntegerCheckMode::Runtime,
            operands: WasmNumericOperationOperands::Binary { left, right },
            scratch: WasmIntegerScratch {
                product: product_scratch,
                power: power_scratch,
            },
        },
    });

    let return_local = if let Some(sum) = source_sum {
        statements.push(WasmLirStmt::CheckedIntegerOp {
            operation: WasmCheckedIntegerOperation {
                dst: sum,
                operator: NumericOperator::Add,
                kind,
                check_mode: IntegerCheckMode::Runtime,
                operands: WasmNumericOperationOperands::Binary { left, right },
                scratch: WasmIntegerScratch {
                    product: None,
                    power: None,
                },
            },
        });
        sum
    } else {
        operation_result
    };
    let function = WasmLirFunction {
        id: function_id,
        debug_name: name.to_owned(),
        origin: WasmLirFunctionOrigin::ExportWrapper,
        signature: WasmLirSignature {
            params: vec![],
            results: vec![carrier],
        },
        locals,
        blocks: vec![WasmLirBlock {
            id: WasmLirBlockId(0),
            statements,
            terminator: WasmLirTerminator::Return {
                value: Some(return_local),
            },
        }],
        linkage: WasmFunctionLinkage::ExportedWrapper,
    };
    let export = WasmExport {
        export_name: name.to_owned(),
        kind: WasmExportKind::Function(function_id),
    };
    (function, export)
}

#[derive(Clone, Copy)]
struct CheckedFloatOperationCase {
    name: &'static str,
    precision: BinaryFloatPrecision,
    operator: NumericOperator,
    left: f64,
    right: Option<f64>,
}

fn checked_float_binary_case(
    name: &'static str,
    precision: BinaryFloatPrecision,
    operator: NumericOperator,
    left: f64,
    right: f64,
) -> CheckedFloatOperationCase {
    CheckedFloatOperationCase {
        name,
        precision,
        operator,
        left,
        right: Some(right),
    }
}

fn checked_float_unary_case(
    name: &'static str,
    precision: BinaryFloatPrecision,
    operator: NumericOperator,
    value: f64,
) -> CheckedFloatOperationCase {
    CheckedFloatOperationCase {
        name,
        precision,
        operator,
        left: value,
        right: None,
    }
}

fn build_checked_float_operation_module(cases: &[CheckedFloatOperationCase]) -> WasmLirModule {
    let mut module = WasmLirModule::default();
    for (function_index, case) in cases.iter().enumerate() {
        let function_id = WasmLirFunctionId(function_index as u32);
        let (function, export) = checked_float_operation_function(function_id, case);
        module.functions.push(function);
        module.exports.push(export);
    }
    module
}

fn build_float_validation_lir_module() -> WasmLirModule {
    let cases = [
        (
            "validate_f32_distinct",
            BinaryFloatPrecision::Binary32,
            false,
        ),
        ("validate_f32_alias", BinaryFloatPrecision::Binary32, true),
        (
            "validate_f64_distinct",
            BinaryFloatPrecision::Binary64,
            false,
        ),
        ("validate_f64_alias", BinaryFloatPrecision::Binary64, true),
    ];
    let mut module = WasmLirModule::default();
    for (index, (name, precision, aliases_source)) in cases.into_iter().enumerate() {
        let function_id = WasmLirFunctionId(index as u32);
        let source = WasmLirLocalId(0);
        let destination = if aliases_source {
            source
        } else {
            WasmLirLocalId(1)
        };
        let carrier = match precision {
            BinaryFloatPrecision::Binary32 => WasmAbiType::F32,
            BinaryFloatPrecision::Binary64 => WasmAbiType::F64,
            BinaryFloatPrecision::Binary16 => panic!("Float validation requires F32 or F64"),
        };
        let mut locals = vec![WasmLirLocal {
            id: source,
            name: Some("source".to_owned()),
            ty: carrier,
            role: WasmLocalRole::Param,
        }];
        if !aliases_source {
            locals.push(local(destination.0, carrier, "validated"));
        }
        module.functions.push(WasmLirFunction {
            id: function_id,
            debug_name: name.to_owned(),
            origin: WasmLirFunctionOrigin::ExportWrapper,
            signature: WasmLirSignature {
                params: vec![carrier],
                results: vec![carrier],
            },
            locals,
            blocks: vec![WasmLirBlock {
                id: WasmLirBlockId(0),
                statements: vec![WasmLirStmt::ValidateFloat {
                    dst: destination,
                    source,
                    precision,
                }],
                terminator: WasmLirTerminator::Return {
                    value: Some(destination),
                },
            }],
            linkage: WasmFunctionLinkage::ExportedWrapper,
        });
        module.exports.push(WasmExport {
            export_name: name.to_owned(),
            kind: WasmExportKind::Function(function_id),
        });
    }
    module
}

fn checked_float_operation_function(
    function_id: WasmLirFunctionId,
    case: &CheckedFloatOperationCase,
) -> (WasmLirFunction, WasmExport) {
    let left = WasmLirLocalId(0);
    let right = WasmLirLocalId(1);
    let result = WasmLirLocalId(2);
    let (carrier, left_constant) = match case.precision {
        BinaryFloatPrecision::Binary32 => (
            WasmAbiType::F32,
            WasmLirStmt::ConstF32 {
                dst: left,
                value: case.left as f32,
            },
        ),
        BinaryFloatPrecision::Binary64 => (
            WasmAbiType::F64,
            WasmLirStmt::ConstF64 {
                dst: left,
                value: case.left,
            },
        ),
        BinaryFloatPrecision::Binary16 => panic!("checked float cases require F32 or F64"),
    };
    let right_constant = case.right.map(|value| match case.precision {
        BinaryFloatPrecision::Binary32 => WasmLirStmt::ConstF32 {
            dst: right,
            value: value as f32,
        },
        BinaryFloatPrecision::Binary64 => WasmLirStmt::ConstF64 { dst: right, value },
        BinaryFloatPrecision::Binary16 => panic!("checked float cases require F32 or F64"),
    });
    let operands = if case.right.is_some() {
        WasmNumericOperationOperands::Binary { left, right }
    } else {
        WasmNumericOperationOperands::Unary { operand: left }
    };
    let mut statements = vec![left_constant];
    if let Some(right_constant) = right_constant {
        statements.push(right_constant);
    }
    statements.push(WasmLirStmt::CheckedFloatOp {
        dst: result,
        operator: case.operator,
        precision: case.precision,
        operands,
    });
    let function = WasmLirFunction {
        id: function_id,
        debug_name: case.name.to_owned(),
        origin: WasmLirFunctionOrigin::ExportWrapper,
        signature: WasmLirSignature {
            params: vec![],
            results: vec![carrier],
        },
        locals: vec![
            local(left.0, carrier, "left"),
            local(right.0, carrier, "right"),
            local(result.0, carrier, "result"),
        ],
        blocks: vec![WasmLirBlock {
            id: WasmLirBlockId(0),
            statements,
            terminator: WasmLirTerminator::Return {
                value: Some(result),
            },
        }],
        linkage: WasmFunctionLinkage::ExportedWrapper,
    };
    let export = WasmExport {
        export_name: case.name.to_owned(),
        kind: WasmExportKind::Function(function_id),
    };
    (function, export)
}

fn build_round_f16_lir_module() -> WasmLirModule {
    let source = WasmLirLocalId(0);
    let rounded = WasmLirLocalId(1);
    let round_function = WasmLirFunction {
        id: WasmLirFunctionId(0),
        debug_name: "round_f16".to_owned(),
        origin: WasmLirFunctionOrigin::ExportWrapper,
        signature: WasmLirSignature {
            params: vec![WasmAbiType::F32],
            results: vec![WasmAbiType::F32],
        },
        locals: vec![
            WasmLirLocal {
                id: source,
                name: Some("source".to_owned()),
                ty: WasmAbiType::F32,
                role: WasmLocalRole::Param,
            },
            local(rounded.0, WasmAbiType::F32, "rounded"),
        ],
        blocks: vec![WasmLirBlock {
            id: WasmLirBlockId(0),
            statements: vec![WasmLirStmt::RoundF16 {
                dst: rounded,
                source,
            }],
            terminator: WasmLirTerminator::Return {
                value: Some(rounded),
            },
        }],
        linkage: WasmFunctionLinkage::ExportedWrapper,
    };

    let left = WasmLirLocalId(0);
    let right = WasmLirLocalId(1);
    let sum = WasmLirLocalId(2);
    let f32_add_function = WasmLirFunction {
        id: WasmLirFunctionId(1),
        debug_name: "f32_add_without_half_rounding".to_owned(),
        origin: WasmLirFunctionOrigin::ExportWrapper,
        signature: WasmLirSignature {
            params: vec![],
            results: vec![WasmAbiType::F32],
        },
        locals: vec![
            local(left.0, WasmAbiType::F32, "left"),
            local(right.0, WasmAbiType::F32, "right"),
            local(sum.0, WasmAbiType::F32, "sum"),
        ],
        blocks: vec![WasmLirBlock {
            id: WasmLirBlockId(0),
            statements: vec![
                WasmLirStmt::ConstF32 {
                    dst: left,
                    value: 1.0,
                },
                WasmLirStmt::ConstF32 {
                    dst: right,
                    value: 2f32.powi(-11),
                },
                WasmLirStmt::CheckedFloatOp {
                    dst: sum,
                    operator: NumericOperator::Add,
                    precision: BinaryFloatPrecision::Binary32,
                    operands: WasmNumericOperationOperands::Binary { left, right },
                },
            ],
            terminator: WasmLirTerminator::Return { value: Some(sum) },
        }],
        linkage: WasmFunctionLinkage::ExportedWrapper,
    };

    WasmLirModule {
        functions: vec![round_function, f32_add_function],
        imports: vec![],
        exports: vec![
            WasmExport {
                export_name: "round".to_owned(),
                kind: WasmExportKind::Function(WasmLirFunctionId(0)),
            },
            WasmExport {
                export_name: "f32_add".to_owned(),
                kind: WasmExportKind::Function(WasmLirFunctionId(1)),
            },
        ],
        static_data: vec![],
        memory_plan: WasmMemoryPlan::default(),
    }
}

fn build_scalar_storage_lir_module() -> WasmLirModule {
    let address = WasmLirLocalId(0);
    let fixed_kind = |scalar| {
        WasmScalarStorageKind::from_fixed_scalar(scalar)
            .expect("fixture uses supported fixed scalar storage")
    };
    let i8_kind = fixed_kind(FixedScalar::I8);
    let u8_kind = fixed_kind(FixedScalar::U8);
    let i16_kind = fixed_kind(FixedScalar::I16);
    let u16_kind = fixed_kind(FixedScalar::U16);
    let i32_kind = fixed_kind(FixedScalar::I32);
    let u32_kind = fixed_kind(FixedScalar::U32);
    let u64_kind = fixed_kind(FixedScalar::U64);
    let f64_kind = fixed_kind(FixedScalar::F64);
    let i64_kind = fixed_kind(FixedScalar::I64);
    let f32_kind = fixed_kind(FixedScalar::F32);
    let f16_kind = fixed_kind(FixedScalar::F16);

    let one_byte_start = 0;
    let two_byte_start = 3;
    let four_byte_start = 11;
    let eight_byte_start = 31;
    let mut statements = vec![
        WasmLirStmt::ConstI32 {
            dst: address,
            value: 1,
        },
        WasmLirStmt::ConstI32 {
            dst: WasmLirLocalId(1),
            value: -2,
        },
        WasmLirStmt::ConstI32 {
            dst: WasmLirLocalId(2),
            value: 254,
        },
        WasmLirStmt::ConstI32 {
            dst: WasmLirLocalId(3),
            value: -0x1234,
        },
        WasmLirStmt::ConstI32 {
            dst: WasmLirLocalId(4),
            value: 0xfedc,
        },
        WasmLirStmt::ConstI32 {
            dst: WasmLirLocalId(5),
            value: -0x0123_4567,
        },
        WasmLirStmt::ConstI32 {
            dst: WasmLirLocalId(6),
            value: 0xfedc_ba98u32 as i32,
        },
        WasmLirStmt::ConstF32 {
            dst: WasmLirLocalId(7),
            value: -0.0,
        },
        WasmLirStmt::ConstI64 {
            dst: WasmLirLocalId(8),
            value: -0x0123_4567_89ab_cdef,
        },
        WasmLirStmt::ConstI64 {
            dst: WasmLirLocalId(9),
            value: 0xfedc_ba98_7654_3210u64 as i64,
        },
        WasmLirStmt::ConstF64 {
            dst: WasmLirLocalId(10),
            value: -0.0,
        },
        WasmLirStmt::ConstF32 {
            dst: WasmLirLocalId(21),
            value: f32::from_bits(0x3380_0000),
        },
        WasmLirStmt::ConstF32 {
            dst: WasmLirLocalId(22),
            value: 65504.0,
        },
        WasmLirStmt::ConstI32 {
            dst: WasmLirLocalId(23),
            value: 0xa55a,
        },
    ];

    statements.extend([
        scalar_store(1, one_byte_start, i8_kind),
        scalar_store(2, one_byte_start + i8_kind.stride(), u8_kind),
        scalar_store(3, two_byte_start, i16_kind),
        scalar_store(4, two_byte_start + i16_kind.stride(), u16_kind),
        scalar_store(5, four_byte_start, i32_kind),
        scalar_store(6, four_byte_start + i32_kind.stride(), u32_kind),
        scalar_store(7, four_byte_start + 2 * i32_kind.stride(), f32_kind),
        scalar_store(8, eight_byte_start, i64_kind),
        scalar_store(9, eight_byte_start + i64_kind.stride(), u64_kind),
        scalar_store(10, eight_byte_start + 2 * i64_kind.stride(), f64_kind),
    ]);
    statements.extend([
        scalar_store(7, 119, f16_kind),
        scalar_store(21, 121, f16_kind),
        scalar_store(22, 123, f16_kind),
        scalar_store(23, 125, u16_kind),
    ]);
    statements.extend([
        scalar_load(11, one_byte_start, i8_kind),
        scalar_load(12, one_byte_start + i8_kind.stride(), u8_kind),
        scalar_load(13, two_byte_start, i16_kind),
        scalar_load(14, two_byte_start + i16_kind.stride(), u16_kind),
        scalar_load(15, four_byte_start, i32_kind),
        scalar_load(16, four_byte_start + i32_kind.stride(), u32_kind),
        scalar_load(17, four_byte_start + 2 * i32_kind.stride(), f32_kind),
        scalar_load(18, eight_byte_start, i64_kind),
        scalar_load(19, eight_byte_start + i64_kind.stride(), u64_kind),
        scalar_load(20, eight_byte_start + 2 * i64_kind.stride(), f64_kind),
    ]);
    statements.extend([
        scalar_load(24, 119, f16_kind),
        scalar_load(25, 121, f16_kind),
        scalar_load(26, 123, f16_kind),
        scalar_load(27, 125, u16_kind),
    ]);
    statements.extend([
        scalar_store(11, 63, u32_kind),
        scalar_store(12, 67, u32_kind),
        scalar_store(13, 71, u32_kind),
        scalar_store(14, 75, u32_kind),
        scalar_store(15, 79, i32_kind),
        scalar_store(16, 83, u32_kind),
        scalar_store(18, 87, i64_kind),
        scalar_store(19, 95, u64_kind),
        scalar_store(17, 103, f32_kind),
        scalar_store(20, 111, f64_kind),
    ]);
    statements.extend([
        scalar_store(24, 127, f16_kind),
        scalar_store(25, 129, f16_kind),
        scalar_store(26, 131, f16_kind),
        scalar_store(27, 133, u16_kind),
        scalar_store(24, 139, f32_kind),
        scalar_store(25, 143, f32_kind),
        scalar_store(26, 147, f32_kind),
    ]);

    let locals = vec![
        local(0, WasmAbiType::I32, "address"),
        local(1, WasmAbiType::I32, "source_i8"),
        local(2, WasmAbiType::I32, "source_u8"),
        local(3, WasmAbiType::I32, "source_i16"),
        local(4, WasmAbiType::I32, "source_u16"),
        local(5, WasmAbiType::I32, "source_i32"),
        local(6, WasmAbiType::I32, "source_u32"),
        local(7, WasmAbiType::F32, "source_f32"),
        local(8, WasmAbiType::I64, "source_i64"),
        local(9, WasmAbiType::I64, "source_u64"),
        local(10, WasmAbiType::F64, "source_f64"),
        local(11, WasmAbiType::I32, "loaded_i8"),
        local(12, WasmAbiType::I32, "loaded_u8"),
        local(13, WasmAbiType::I32, "loaded_i16"),
        local(14, WasmAbiType::I32, "loaded_u16"),
        local(15, WasmAbiType::I32, "loaded_i32"),
        local(16, WasmAbiType::I32, "loaded_u32"),
        local(17, WasmAbiType::F32, "loaded_f32"),
        local(18, WasmAbiType::I64, "loaded_i64"),
        local(19, WasmAbiType::I64, "loaded_u64"),
        local(20, WasmAbiType::F64, "loaded_f64"),
        local(21, WasmAbiType::F32, "source_f16_subnormal"),
        local(22, WasmAbiType::F32, "source_f16_max"),
        local(23, WasmAbiType::I32, "adjacent_sentinel"),
        local(24, WasmAbiType::F32, "loaded_f16_zero"),
        local(25, WasmAbiType::F32, "loaded_f16_subnormal"),
        local(26, WasmAbiType::F32, "loaded_f16_max"),
        local(27, WasmAbiType::I32, "loaded_adjacent_sentinel"),
    ];
    WasmLirModule {
        functions: vec![WasmLirFunction {
            id: WasmLirFunctionId(0),
            debug_name: "scalar_storage_round_trip".to_owned(),
            origin: WasmLirFunctionOrigin::ExportWrapper,
            signature: WasmLirSignature {
                params: vec![],
                results: vec![],
            },
            locals,
            blocks: vec![WasmLirBlock {
                id: WasmLirBlockId(0),
                statements,
                terminator: WasmLirTerminator::Return { value: None },
            }],
            linkage: WasmFunctionLinkage::ExportedWrapper,
        }],
        imports: vec![],
        exports: vec![WasmExport {
            export_name: "run".to_owned(),
            kind: WasmExportKind::Function(WasmLirFunctionId(0)),
        }],
        static_data: vec![WasmStaticData {
            id: WasmStaticDataId(0),
            debug_name: "scalar_storage_sentinels".to_owned(),
            bytes: vec![b'~'; 64],
            kind: WasmStaticDataKind::Utf8StringBytes,
        }],
        memory_plan: WasmMemoryPlan::default(),
    }
}

fn build_integer_string_conversion_lir_module() -> WasmLirModule {
    let cases = [
        ("signed_i32", WasmAbiType::I32, true),
        ("unsigned_i32", WasmAbiType::I32, false),
        ("signed_i64", WasmAbiType::I64, true),
        ("unsigned_i64", WasmAbiType::I64, false),
    ];
    let mut functions = Vec::with_capacity(cases.len());
    let mut exports = Vec::with_capacity(cases.len());
    for (index, (name, input_type, signed)) in cases.into_iter().enumerate() {
        let (function, export) =
            integer_string_conversion_function(index as u32, name, input_type, signed);
        functions.push(function);
        exports.push(export);
    }
    WasmLirModule {
        functions,
        imports: vec![],
        exports,
        static_data: vec![],
        memory_plan: WasmMemoryPlan::default(),
    }
}

fn integer_string_conversion_function(
    function_id: u32,
    export_name: &str,
    input_type: WasmAbiType,
    signed: bool,
) -> (WasmLirFunction, WasmExport) {
    let input = WasmLirLocalId(0);
    let output = WasmLirLocalId(1);
    let conversion = if signed {
        WasmLirStmt::StringFromI64 {
            dst: output,
            value: input,
        }
    } else {
        WasmLirStmt::StringFromU64 {
            dst: output,
            value: input,
        }
    };
    let id = WasmLirFunctionId(function_id);
    let function = WasmLirFunction {
        id,
        debug_name: format!("integer_to_string_{export_name}"),
        origin: WasmLirFunctionOrigin::ExportWrapper,
        signature: WasmLirSignature {
            params: vec![input_type],
            results: vec![WasmAbiType::Handle],
        },
        locals: vec![
            WasmLirLocal {
                id: input,
                name: Some("value".to_owned()),
                ty: input_type,
                role: WasmLocalRole::Param,
            },
            local(output.0, WasmAbiType::Handle, "string_handle"),
        ],
        blocks: vec![WasmLirBlock {
            id: WasmLirBlockId(0),
            statements: vec![conversion],
            terminator: WasmLirTerminator::Return {
                value: Some(output),
            },
        }],
        linkage: WasmFunctionLinkage::ExportedWrapper,
    };
    let export = WasmExport {
        export_name: export_name.to_owned(),
        kind: WasmExportKind::Function(id),
    };
    (function, export)
}

fn build_float_string_conversion_lir_module() -> WasmLirModule {
    let cases = [
        (
            "string_f16",
            WasmAbiType::F32,
            BinaryFloatPrecision::Binary16,
        ),
        (
            "string_f32",
            WasmAbiType::F32,
            BinaryFloatPrecision::Binary32,
        ),
        (
            "string_f64",
            WasmAbiType::F64,
            BinaryFloatPrecision::Binary64,
        ),
    ];
    let mut functions = Vec::with_capacity(cases.len());
    let mut exports = Vec::with_capacity(cases.len());
    for (index, (name, carrier, precision)) in cases.into_iter().enumerate() {
        let function_id = WasmLirFunctionId(index as u32);
        let input = WasmLirLocalId(0);
        let output = WasmLirLocalId(1);
        functions.push(WasmLirFunction {
            id: function_id,
            debug_name: format!("float_to_string_{name}"),
            origin: WasmLirFunctionOrigin::ExportWrapper,
            signature: WasmLirSignature {
                params: vec![carrier],
                results: vec![WasmAbiType::Handle],
            },
            locals: vec![
                WasmLirLocal {
                    id: input,
                    name: Some("value".to_owned()),
                    ty: carrier,
                    role: WasmLocalRole::Param,
                },
                local(output.0, WasmAbiType::Handle, "string_handle"),
            ],
            blocks: vec![WasmLirBlock {
                id: WasmLirBlockId(0),
                statements: vec![WasmLirStmt::StringFromFloat {
                    dst: output,
                    value: input,
                    precision,
                }],
                terminator: WasmLirTerminator::Return {
                    value: Some(output),
                },
            }],
            linkage: WasmFunctionLinkage::ExportedWrapper,
        });
        exports.push(WasmExport {
            export_name: name.to_owned(),
            kind: WasmExportKind::Function(function_id),
        });
    }
    WasmLirModule {
        functions,
        imports: vec![],
        exports,
        static_data: vec![],
        memory_plan: WasmMemoryPlan::default(),
    }
}

fn float_text_f32_cases() -> Vec<u32> {
    let mut bits = vec![
        0x0000_0000,
        0x8000_0000,
        0x0000_0001,
        0x8000_0001,
        0x007f_ffff,
        0x807f_ffff,
        0x0080_0000,
        0x8080_0000,
        0x7f7f_ffff,
        0xff7f_ffff,
        0x3d_cc_cc_cd, // 0.1, also used by the promoted-precision comparison.
        0x477f_e000,   // 65504, the largest finite F16 value.
    ];
    for value in [1.0e-6f32, 1.0e21f32] {
        push_f32_neighbor_bits(&mut bits, value.to_bits());
    }
    for exponent in [-126, -14, -1, 0, 1, 23, 127] {
        push_f32_neighbor_bits(&mut bits, 2.0f32.powi(exponent).to_bits());
    }

    let mut state = 0x5a17_39cdu32;
    let mut random_count = 0;
    while random_count < 128 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        if state & 0x7f80_0000 != 0x7f80_0000 {
            bits.push(state);
            random_count += 1;
        }
    }
    bits
}

fn push_f32_neighbor_bits(bits: &mut Vec<u32>, center: u32) {
    for neighbor in [center - 1, center, center + 1] {
        bits.push(neighbor);
        bits.push(neighbor | 0x8000_0000);
    }
}

fn float_text_f64_cases() -> Vec<u64> {
    let mut bits = vec![
        0x0000_0000_0000_0000,
        0x8000_0000_0000_0000,
        0x0000_0000_0000_0001,
        0x8000_0000_0000_0001,
        0x000f_ffff_ffff_ffff,
        0x800f_ffff_ffff_ffff,
        0x0010_0000_0000_0000,
        0x8010_0000_0000_0000,
        0x7fef_ffff_ffff_ffff,
        0xffef_ffff_ffff_ffff,
    ];
    for value in [1.0e-6f64, 1.0e21f64] {
        push_f64_neighbor_bits(&mut bits, value.to_bits());
    }
    for exponent in [-1022, -1, 0, 1, 52, 1023] {
        push_f64_neighbor_bits(&mut bits, 2.0f64.powi(exponent).to_bits());
    }
    bits.push(f64::from(f32::from_bits(0x3dcc_cccd)).to_bits());

    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut random_count = 0;
    while random_count < 128 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        if state & 0x7ff0_0000_0000_0000 != 0x7ff0_0000_0000_0000 {
            bits.push(state);
            random_count += 1;
        }
    }
    bits
}

fn push_f64_neighbor_bits(bits: &mut Vec<u64>, center: u64) {
    for neighbor in [center - 1, center, center + 1] {
        bits.push(neighbor);
        bits.push(neighbor | 0x8000_0000_0000_0000);
    }
}

fn binary16_bits_as_f64(bits: u16) -> f64 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let fraction = f64::from(bits & 0x03ff);
    if exponent == 0 {
        sign * fraction * 2.0f64.powi(-24)
    } else {
        sign * (1.0 + fraction / 1024.0) * 2.0f64.powi(exponent - 15)
    }
}

fn push_float_text_expectation(
    expected: &mut Vec<String>,
    precision_name: &str,
    bits: String,
    value: f64,
    precision: BinaryFloatPrecision,
) {
    let text = format_finite_float(value, precision)
        .unwrap_or_else(|error| panic!("{precision_name} bits {bits}: {error}"));
    expected.push(format!("{precision_name}|{bits}|{text}"));
}

fn execute_wasm_checked_exports_in_node(
    wasm_bytes: &[u8],
    export_specs: &[(&str, Option<BinaryFloatPrecision>)],
) -> Vec<(String, String, String)> {
    const NODE_BODY: &str = r#"
const results = [];
for (const [name, precision] of __EXPORT_SPECS__) {
  try {
    const value = instance.exports[name]();
    let rendered = String(value);
    if (precision === "f32") {
      const view = new DataView(new ArrayBuffer(4));
      view.setFloat32(0, value, true);
      rendered = view.getUint32(0, true).toString(16).padStart(8, "0");
    } else if (precision === "f64") {
      const view = new DataView(new ArrayBuffer(8));
      view.setFloat64(0, value, true);
      rendered = view.getBigUint64(0, true).toString(16).padStart(16, "0");
    }
    results.push(`${name}|ok|${rendered}`);
  } catch (error) {
    if (error !== null
        && typeof error === "object"
        && Object.getPrototypeOf(error) === WebAssembly.RuntimeError.prototype) {
      results.push(`${name}|trap|${String(error.message)}`);
    } else {
      throw error;
    }
  }
}
process.stdout.write(results.join("\n"));
"#;
    let export_specs = export_specs
        .iter()
        .map(|(name, precision)| {
            let precision = match precision {
                Some(BinaryFloatPrecision::Binary32) => "\"f32\"",
                Some(BinaryFloatPrecision::Binary64) => "\"f64\"",
                Some(BinaryFloatPrecision::Binary16) => {
                    panic!("checked float observations require F32 or F64")
                }
                None => "null",
            };
            format!("[{name:?}, {precision}]")
        })
        .collect::<Vec<_>>()
        .join(", ");
    let node_body = NODE_BODY.replace("__EXPORT_SPECS__", &format!("[{export_specs}]"));
    let output = run_wasm_node_script(wasm_bytes, "", &node_body);
    String::from_utf8(output)
        .expect("Node checked Wasm results should be UTF-8")
        .lines()
        .map(|line| {
            let mut parts = line.splitn(3, '|');
            (
                parts
                    .next()
                    .expect("result should include export name")
                    .to_owned(),
                parts
                    .next()
                    .expect("result should include outcome")
                    .to_owned(),
                parts
                    .next()
                    .expect("result should include value or trap")
                    .to_owned(),
            )
        })
        .collect()
}

fn execute_wasm_float_validation_in_node(
    wasm_bytes: &[u8],
    export_specs: &[(&str, BinaryFloatPrecision, &[&str])],
) -> Vec<String> {
    const NODE_BODY: &str = r#"
const results = [];
const view = new DataView(new ArrayBuffer(8));
for (const [name, precision, inputBits] of __EXPORT_SPECS__) {
  for (const bits of inputBits) {
    let input;
    if (precision === "f32" && bits.length === 16) {
      // The host ABI converts a finite f64 argument to the f32 parameter before validation.
      view.setBigUint64(0, BigInt(`0x${bits}`), true);
      input = view.getFloat64(0, true);
    } else if (precision === "f32") {
      view.setUint32(0, Number.parseInt(bits, 16), true);
      input = view.getFloat32(0, true);
    } else {
      view.setBigUint64(0, BigInt(`0x${bits}`), true);
      input = view.getFloat64(0, true);
    }
    try {
      const value = instance.exports[name](input);
      let rendered;
      if (precision === "f32") {
        view.setFloat32(0, value, true);
        rendered = view.getUint32(0, true).toString(16).padStart(8, "0");
      } else {
        view.setFloat64(0, value, true);
        rendered = view.getBigUint64(0, true).toString(16).padStart(16, "0");
      }
      results.push(`${name}|${bits}|ok|${rendered}`);
    } catch (error) {
      if (error !== null
          && typeof error === "object"
          && Object.getPrototypeOf(error) === WebAssembly.RuntimeError.prototype) {
        results.push(`${name}|${bits}|trap|${String(error.message)}`);
      } else {
        throw error;
      }
    }
  }
}
process.stdout.write(results.join("\n"));
"#;
    let export_specs = export_specs
        .iter()
        .map(|(name, precision, input_bits)| {
            let precision = match precision {
                BinaryFloatPrecision::Binary32 => "\"f32\"",
                BinaryFloatPrecision::Binary64 => "\"f64\"",
                BinaryFloatPrecision::Binary16 => {
                    panic!("Float validation observations require F32 or F64")
                }
            };
            let input_bits = input_bits
                .iter()
                .map(|bits| format!("{bits:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{name:?}, {precision}, [{input_bits}]]")
        })
        .collect::<Vec<_>>()
        .join(", ");
    let node_body = NODE_BODY.replace("__EXPORT_SPECS__", &format!("[{export_specs}]"));
    let output = run_wasm_node_script(wasm_bytes, "", &node_body);
    String::from_utf8(output)
        .expect("Node Float validation results should be UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn scalar_load(dst: u32, offset: u32, kind: WasmScalarStorageKind) -> WasmLirStmt {
    WasmLirStmt::LoadScalar {
        dst: WasmLirLocalId(dst),
        address: WasmLirLocalId(0),
        offset,
        kind,
    }
}

fn scalar_store(value: u32, offset: u32, kind: WasmScalarStorageKind) -> WasmLirStmt {
    WasmLirStmt::StoreScalar {
        address: WasmLirLocalId(0),
        offset,
        value: WasmLirLocalId(value),
        kind,
    }
}

fn execute_wasm_in_node(wasm_bytes: &[u8]) -> Vec<u8> {
    const NODE_BODY: &str = r#"
instance.exports.run();
process.stdout.write(Buffer.from(instance.exports.memory.buffer, 0, 152));
"#;
    run_wasm_node_script(wasm_bytes, "", NODE_BODY)
}

fn run_wasm_node_script(wasm_bytes: &[u8], node_setup: &str, node_body: &str) -> Vec<u8> {
    const NODE_SCRIPT: &str = r#"
const chunks = [];
process.stdin.on("data", chunk => chunks.push(chunk));
process.stdin.on("end", async () => {
  try {
    const bytes = Buffer.concat(chunks);
    const imports = {};
    __NODE_SETUP__
    const { instance } = await WebAssembly.instantiate(bytes, imports);
    __NODE_BODY__
  } catch (error) {
    console.error(error && error.stack ? error.stack : error);
    process.exitCode = 1;
  }
});
"#;
    let node_script = NODE_SCRIPT
        .replace("__NODE_SETUP__", node_setup)
        .replace("__NODE_BODY__", node_body);
    let mut child = Command::new("node")
        .args(["--eval", node_script.as_str()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Node must be available for Wasm execution tests");
    child
        .stdin
        .take()
        .expect("Node stdin should be piped")
        .write_all(wasm_bytes)
        .expect("emitted Wasm bytes should reach Node");
    let output = child
        .wait_with_output()
        .expect("Node execution should complete");
    assert!(
        output.status.success(),
        "Node Wasm execution failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// Node runner for the fixture's `run(a, b, d)`, `run_i64()` and `run_u64()` exports.
///
/// Returns one `a,b,d|ok|value` or `a,b,d|trap|message` line per case, preserving case order,
/// followed by fixed `i64=<value>` and `u64=<value>` lines from the width exports.
fn execute_wasm_proof_run_in_node(wasm_bytes: &[u8], cases: &[[i64; 3]]) -> Vec<String> {
    const NODE_BODY: &str = r#"
const results = [];
for (const [a, b, d] of __CASES__) {
  try {
    const value = instance.exports.run(a, b, d);
    results.push(`${a},${b},${d}|ok|${value}`);
  } catch (error) {
    if (error !== null
        && typeof error === "object"
        && Object.getPrototypeOf(error) === WebAssembly.RuntimeError.prototype) {
      results.push(`${a},${b},${d}|trap|${String(error.message)}`);
    } else {
      throw error;
    }
  }
}
results.push(`i64=${instance.exports.run_i64()}`);
results.push(`u64=${instance.exports.run_u64()}`);
process.stdout.write(results.join("\n"));
"#;
    let cases = cases
        .iter()
        .map(|case| format!("[{}, {}, {}]", case[0], case[1], case[2]))
        .collect::<Vec<_>>()
        .join(", ");
    let node_body = NODE_BODY.replace("__CASES__", &format!("[{cases}]"));
    let output = run_wasm_node_script(wasm_bytes, "", &node_body);
    String::from_utf8(output)
        .expect("Node proof-run results should be UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Executes the retained and analysed editions of the shared proof fixture in Node.
///
/// WHAT: lowers the fixture twice (empty table vs `analyse_numeric_proofs` table of the same
///       HIR), emits both modules, and asserts:
///       - both editions validate as Wasm,
///       - the exact checksum flows from the proven x alias add/multiply chain through the
///         retained q divide into the safe-case return value 44; `run_i64` (proven I64 add
///         alias chain) and `run_u64` (proven U64 multiply) report their exact values, while
///         the run() U64 divide statement proves only for its exact non-zero divisor form
///         and the dynamic q divide stays retained,
///       - retained-only checks keep their traps for subtraction underflow, dynamic negation of
///         INT_MIN, division by zero and the checksum alias ADD overflow (i32::MAX input),
///       - the editions agree on every case (same values, same trap statuses),
///       - the analysed bytes differ from retained bytes (predicates and check-only scratch are
///         actually gone).
/// WHY: proof-mode emission must be behavior-identical on real instantiated code except for the
///       checks the analysis excluded.
#[test]
fn proven_integer_operations_execute_with_aliased_destinations() {
    let fixture =
        crate::backends::wasm::tests::lowering::numeric_proof_tests::build_proof_fixture_hir();
    let mut request = WasmBackendRequest::default();
    request.emit_options.validate_emitted_module = true;
    request.export_policy = WasmExportPolicy {
        exported_functions: vec![FunctionId(0), FunctionId(1), FunctionId(2)],
        export_names: {
            let mut export_names = FxHashMap::default();
            export_names.insert(FunctionId(0), "run".to_owned());
            export_names.insert(FunctionId(1), "run_i64".to_owned());
            export_names.insert(FunctionId(2), "run_u64".to_owned());
            export_names
        },
        helper_exports: Default::default(),
    };

    let lower = |proofs: &crate::compiler_frontend::analysis::numeric_proofs::NumericProofs| {
        lower_hir_to_wasm_module(
            &fixture.module,
            &default_borrow_facts(),
            proofs,
            &request,
            &fixture.string_table,
            &fixture.type_environment,
            &fixture.path_table,
        )
        .expect("proof fixture lowering should emit")
    };
    let retained = lower(&default_numeric_proofs());
    let analysed = lower(&analyse_numeric_proofs(
        &fixture.module,
        &fixture.type_environment,
        NumericProfile::STANDARD,
    ));

    let retained_bytes = retained.wasm_bytes.expect("retained edition emits bytes");
    let analysed_bytes = analysed.wasm_bytes.expect("analysed edition emits bytes");
    validate_wasm(&retained_bytes);
    validate_wasm(&analysed_bytes);
    assert_ne!(
        retained_bytes, analysed_bytes,
        "the excluded checks and check-only scratch must actually change emitted code"
    );

    // (a, b, d): the safe case keeps every dynamic statement inside bounds; the trap cases each
    // exercise one retained-only failure (subtraction underflow, INT_MIN negation, zero divisor,
    // and the checksum add overflowing through a large `a`).
    let ok_case = [0, 7, 6];
    let negation_trap_case = [0, i32::MIN as i64, 1];
    let division_trap_case = [0, 5, 0];
    let underflow_trap_case = [i32::MIN as i64, 5, 1];
    let overflow_trap_case = [i32::MAX as i64, 7, 6];
    let all_cases = [
        ok_case,
        negation_trap_case,
        division_trap_case,
        underflow_trap_case,
        overflow_trap_case,
    ];

    let retained_results = execute_wasm_proof_run_in_node(&retained_bytes, &all_cases);
    let analysed_results = execute_wasm_proof_run_in_node(&analysed_bytes, &all_cases);

    assert_eq!(
        retained_results, analysed_results,
        "both editions must observe identical values and trap statuses per case"
    );
    // checksum consumption: ((36 + 6) + (0 - 6)) + 8 = 44, so the exact safe x chain reaches
    // the export through the proof-mode lowering and its dynamic consumers.
    assert_eq!(
        retained_results[0], "0,7,6|ok|44",
        "the safe case must return the checksum computed through the safe chain and dynamic \
         consumers"
    );
    assert_eq!(retained_results.last(), Some(&"u64=36".to_owned()));
    assert_eq!(retained_results[retained_results.len() - 2], "i64=6");
    for (index, result) in retained_results.iter().enumerate().skip(1).take(4) {
        // The Node runner emits `a,b,d|trap|<RuntimeError message>`; assert the exact case
        // input prefix and the parsed status column, not the incidental engine message.
        let case = all_cases[index];
        let expected_prefix = format!("{},{},{}|", case[0], case[1], case[2]);
        assert!(
            result.starts_with(&expected_prefix),
            "each Node result must name its exact case input: {result}"
        );
        let status = result.split('|').nth(1);
        assert_eq!(status, Some("trap"), "the case must trap: {result}");
    }
    assert_eq!(
        retained_results.len(),
        7,
        "node returned every expected line"
    );
}

/// Instantiated Uint scalar execution for the Wasm backend.
///
/// WHAT: lowers one HIR module with Uint parameters, locals and results through both integer
///       profiles, instantiates the emitted bytes in Node and asserts exact values and trap
///       statuses for unsigned arithmetic, exact Uint/Uint and Uint/Int comparisons and
///       infallible Uint-to-Float conversion including the direct Float32 rounding witness.
/// WHY: trap-mode Uint paths must execute with unsigned semantics and bit-pattern preservation
///      above the signed maximum. Expected Wasm traps stay terminal `RuntimeError` assertions
///      here; Moth `Error` codes belong to the JS/source recovery tests, never to this module.
#[test]
fn uint_scalar_operations_execute_with_unsigned_semantics_in_node() {
    for profile in [uint_test_profile_32(), uint_test_profile_64()] {
        let fixture = build_uint_scalar_fixture(profile);
        let mut request = WasmBackendRequest {
            numeric_profile: profile,
            export_policy: uint_scalar_export_policy(&fixture.names),
            ..WasmBackendRequest::default()
        };
        request.emit_options.validate_emitted_module = true;
        let result = lower_hir_to_wasm_module(
            &fixture.module,
            &default_borrow_facts(),
            &default_numeric_proofs(),
            &request,
            &fixture.string_table,
            &fixture.type_environment,
            &fixture.path_table,
        )
        .unwrap_or_else(|error| panic!("{profile:?} Uint fixture should emit: {error:?}"));
        let bytes = result.wasm_bytes.expect("Uint fixture emits bytes");
        validate_wasm(&bytes);
        let actual = execute_wasm_uint_exports_in_node(&bytes, &uint_scalar_calls(profile));
        for (name, status, value) in uint_scalar_expectations(profile) {
            let found = actual
                .iter()
                .find(|(actual_name, _, _)| actual_name == &name)
                .unwrap_or_else(|| panic!("{profile:?} missing Uint export result for {name}"));
            assert_eq!(
                found.1, status,
                "{profile:?} {name} had an unexpected outcome"
            );
            if status == "trap" {
                // Terminal Wasm traps surface as engine `RuntimeError`s, never as Moth `Error`
                // codes: recovery expectations belong to the JS/source tests, not to this module.
                assert!(
                    !found.2.is_empty(),
                    "{profile:?} {name} should report a trap"
                );
                assert!(
                    !found.2.contains("MOTH-"),
                    "{profile:?} {name} must not surface a Moth Error code: {:?}",
                    found.2
                );
            } else {
                assert_eq!(
                    found.2, value,
                    "{profile:?} {name} returned an unexpected result"
                );
            }
        }
    }
}

#[test]
fn uint_storage_maps_to_compact_carriers_with_natural_stride() {
    let profiles = [
        (
            IntWidth::Bits32,
            FloatPrecision::Bits32,
            WasmScalarStorageKind::I32,
        ),
        (
            IntWidth::Bits32,
            FloatPrecision::Bits64,
            WasmScalarStorageKind::I32,
        ),
        (
            IntWidth::Bits64,
            FloatPrecision::Bits32,
            WasmScalarStorageKind::I64,
        ),
        (
            IntWidth::Bits64,
            FloatPrecision::Bits64,
            WasmScalarStorageKind::I64,
        ),
    ];
    for (int_width, float_precision, expected) in profiles {
        let profile = NumericProfile {
            int_width,
            float_precision,
        };
        let kind = WasmScalarStorageKind::for_numeric_scalar(NumericScalar::Uint, profile)
            .expect("Uint has scalar storage under every profile");
        assert_eq!(kind, expected, "{profile:?} Uint storage");
        let (size, carrier) = match int_width {
            IntWidth::Bits32 => (4, WasmAbiType::I32),
            IntWidth::Bits64 => (8, WasmAbiType::I64),
        };
        assert_eq!(kind.size(), size, "{profile:?} Uint size");
        assert_eq!(kind.alignment(), size, "{profile:?} Uint alignment");
        assert_eq!(kind.stride(), size, "{profile:?} Uint stride");
        assert_eq!(kind.carrier(), carrier, "{profile:?} Uint carrier");
    }
    assert_eq!(
        WasmScalarStorageKind::for_numeric_scalar(NumericScalar::Uint, NumericProfile::STANDARD,),
        WasmScalarStorageKind::for_numeric_scalar(NumericScalar::Int, NumericProfile::STANDARD),
        "Uint shares the Int width carrier without aliasing its semantic identity"
    );
}

fn uint_test_profile_32() -> NumericProfile {
    NumericProfile {
        int_width: IntWidth::Bits32,
        float_precision: FloatPrecision::Bits64,
    }
}

fn uint_test_profile_64() -> NumericProfile {
    NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits64,
    }
}

struct UintScalarFixture {
    module: crate::compiler_frontend::hir::module::HirModule,
    names: Vec<(FunctionId, String)>,
    string_table: StringTable,
    type_environment: crate::compiler_frontend::datatypes::environment::TypeEnvironment,
    path_table: crate::compiler_frontend::symbols::path_interner::PathTable,
}
/// One exported Uint helper per behaviour under test; every arithmetic helper is a trap-mode
/// `NumericOp` over its parameters so Node observes native values or terminal traps. The module
/// keeps a dedicated entry-start `main` following the established start/export split; only the
/// named Uint helpers are exported.
fn build_uint_scalar_fixture(profile: NumericProfile) -> UintScalarFixture {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let uint_type = builtin_type_ids::UINT;
    let int_type = builtin_type_ids::INT;
    let bool_type = builtin_type_ids::BOOL;
    let float_type = builtin_type_ids::FLOAT;
    let f32_type = builtin_type_ids::fixed_scalar(FixedScalar::F32);
    let region = RegionId(0);

    let mut functions = Vec::new();
    let mut blocks = Vec::new();
    let mut names: Vec<(FunctionId, String)> = Vec::new();
    let start_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");
    blocks.push(HirBlock {
        id: BlockId(100),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(9000, 0, types.int, region)),
    });
    functions.push((
        HirFunction {
            id: FunctionId(0),
            entry: BlockId(100),
            params: vec![],
            return_type: types.int,
        },
        start_path,
        HirFunctionOrigin::EntryStart,
    ));
    let mut next_function_id: u32 = 1;
    let mut intern = |name: &str| {
        path_fork
            .try_intern_portable_path(name, &mut string_table)
            .expect("test path fits")
    };
    // Export order fixes the expected-result order asserted below.
    let builders: Vec<(&str, NumericOperator)> = vec![
        ("uadd", NumericOperator::Add),
        ("usub", NumericOperator::Subtract),
        ("umul", NumericOperator::Multiply),
        ("udiv", NumericOperator::IntegerDivide),
        ("urem", NumericOperator::Remainder),
        ("upow", NumericOperator::Power),
    ];
    for (name, operator) in builders.into_iter() {
        let id = FunctionId(next_function_id);
        next_function_id += 1;
        let path = intern(name);
        let block_id = BlockId(1000 + id.0);
        blocks.push(HirBlock {
            id: block_id,
            region,
            locals: vec![
                hir_local(0, uint_type, region),
                hir_local(1, uint_type, region),
                hir_local(2, uint_type, region),
            ],
            statements: vec![hir_statement(
                id.0 * 100 + 10,
                HirStatementKind::NumericOp {
                    op: HirNumericOp {
                        operator,
                        domain: NumericScalar::Uint,
                    },
                    failure_mode: NumericFailureMode::Trap,
                    operands: HirNumericOperands::Binary {
                        left: hir_load_local(11, LocalId(0), uint_type, region),
                        right: hir_load_local(12, LocalId(1), uint_type, region),
                    },
                    result: LocalId(2),
                },
                1,
            )],
            terminator: HirTerminator::Return(hir_load_local(13, LocalId(2), uint_type, region)),
        });
        functions.push((
            HirFunction {
                id,
                entry: block_id,
                params: vec![LocalId(0), LocalId(1)],
                return_type: uint_type,
            },
            path,
            HirFunctionOrigin::Normal,
        ));
        names.push((id, name.to_owned()));
    }
    // Above the signed maximum Uint/Uint order stays unsigned: the signed reinterpretation
    // would invert every assertion below.
    let ordered: Vec<(&str, HirBinOp)> = vec![
        ("ucmp_lt", HirBinOp::Lt),
        ("ucmp_ge", HirBinOp::Ge),
        ("ucmp_eq", HirBinOp::Eq),
    ];
    for (name, operator) in ordered.into_iter() {
        let id = FunctionId(next_function_id);
        next_function_id += 1;
        let path = intern(name);
        let block_id = BlockId(1000 + id.0);
        let body = expression(
            id.0 * 100 + 33,
            HirExpressionKind::BinOp {
                left: Box::new(hir_load_local(31, LocalId(0), uint_type, region)),
                op: operator,
                right: Box::new(hir_load_local(32, LocalId(1), uint_type, region)),
            },
            bool_type,
            region,
            ValueKind::RValue,
        );
        blocks.push(HirBlock {
            id: block_id,
            region,
            locals: vec![
                hir_local(0, uint_type, region),
                hir_local(1, uint_type, region),
            ],
            statements: vec![],
            terminator: HirTerminator::Return(body),
        });
        functions.push((
            HirFunction {
                id,
                entry: block_id,
                params: vec![LocalId(0), LocalId(1)],
                return_type: bool_type,
            },
            path,
            HirFunctionOrigin::Normal,
        ));
        names.push((id, name.to_owned()));
    }
    // A negative Int is less than every Uint, including zero; a Uint above the signed maximum
    // stays greater than the signed maximum in both operand orders.
    let mixed: Vec<(&str, HirBinOp, bool)> = vec![
        ("umix_lt", HirBinOp::Lt, false),
        ("umix_gt", HirBinOp::Gt, true),
    ];
    for (name, operator, left_is_uint) in mixed.into_iter() {
        let id = FunctionId(next_function_id);
        next_function_id += 1;
        let path = intern(name);
        let block_id = BlockId(1000 + id.0);
        let (left_type, right_type) = if left_is_uint {
            (uint_type, int_type)
        } else {
            (int_type, uint_type)
        };
        let body = expression(
            id.0 * 100 + 23,
            HirExpressionKind::BinOp {
                left: Box::new(hir_load_local(21, LocalId(0), left_type, region)),
                op: operator,
                right: Box::new(hir_load_local(22, LocalId(1), right_type, region)),
            },
            bool_type,
            region,
            ValueKind::RValue,
        );
        blocks.push(HirBlock {
            id: block_id,
            region,
            locals: vec![
                hir_local(0, left_type, region),
                hir_local(1, right_type, region),
            ],
            statements: vec![],
            terminator: HirTerminator::Return(body),
        });
        functions.push((
            HirFunction {
                id,
                entry: block_id,
                params: vec![LocalId(0), LocalId(1)],
                return_type: bool_type,
            },
            path,
            HirFunctionOrigin::Normal,
        ));
        names.push((id, name.to_owned()));
    }
    // Infallible Uint-to-Float conversions. The Float32 witness input exceeds the 32-bit Uint
    // range, so it only joins 64-bit profiles; the profile Float conversion is always present.
    // The witness rounds directly to 9007200000000000 (F32 bits 0x5a000001).
    let mut conversions: Vec<(
        &str,
        u64,
        NumericScalar,
        crate::compiler_frontend::datatypes::ids::TypeId,
    )> = vec![("uto_float", 3_000_000_001, NumericScalar::Float, float_type)];
    if profile.int_width == IntWidth::Bits64 {
        conversions.push((
            "uto_f32",
            9_007_199_791_611_905,
            NumericScalar::Fixed(FixedScalar::F32),
            f32_type,
        ));
    }
    for (name, value, target, return_type) in conversions.into_iter() {
        let id = FunctionId(next_function_id);
        next_function_id += 1;
        let path = intern(name);
        let block_id = BlockId(1000 + id.0);
        let source = expression(
            id.0 * 100 + 41,
            HirExpressionKind::Uint(value),
            uint_type,
            region,
            ValueKind::Const,
        );
        let body = expression(
            id.0 * 100 + 42,
            HirExpressionKind::Cast {
                source: Box::new(source),
                policy: crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId::NumericConversion {
                    source: NumericScalar::Uint,
                    target,
                },
            },
            return_type,
            region,
            ValueKind::RValue,
        );
        blocks.push(HirBlock {
            id: block_id,
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(body),
        });
        functions.push((
            HirFunction {
                id,
                entry: block_id,
                params: vec![],
                return_type,
            },
            path,
            HirFunctionOrigin::Normal,
        ));
        names.push((id, name.to_owned()));
    }

    let module = build_module(
        &mut path_fork,
        &mut string_table,
        functions,
        blocks,
        FunctionId(0),
    );
    let _ = next_function_id;
    UintScalarFixture {
        module,
        names,
        string_table,
        type_environment,
        path_table: path_fork.snapshot_table(),
    }
}

fn uint_scalar_export_policy(names: &[(FunctionId, String)]) -> WasmExportPolicy {
    let mut export_names = FxHashMap::default();
    let mut exported_functions = Vec::new();
    for (id, name) in names {
        exported_functions.push(*id);
        export_names.insert(*id, name.clone());
    }
    WasmExportPolicy {
        exported_functions,
        export_names,
        helper_exports: Default::default(),
    }
}

/// Calls use JS Numbers for 32-bit carriers and BigInts for 64-bit carriers; float results
/// render as F32/F64 hex bits so the direct-rounding assertions are exact.
fn uint_scalar_calls(profile: NumericProfile) -> Vec<(String, Vec<String>, Option<String>)> {
    let big = profile.int_width == IntWidth::Bits64;
    // Renders one Wasm integer parameter in its JS call shape.
    let param = |value: &str| {
        if big {
            format!("{value}n")
        } else {
            value.to_owned()
        }
    };
    let mut calls = Vec::new();
    let arith: Vec<(&str, &str, &str)> = if big {
        vec![
            ("uadd", "9223372036854775808", "100"),
            ("usub", "9223372036854775808", "1"),
            ("umul", "1000000000", "18446744073"),
            ("udiv", "18446744073709551615", "9223372036854775808"),
            ("urem", "18446744073709551615", "9223372036854775808"),
            ("upow", "2", "10"),
        ]
    } else {
        vec![
            ("uadd", "3000000001", "1000000000"),
            ("usub", "5", "3"),
            ("umul", "65535", "65537"),
            ("udiv", "7", "2"),
            ("urem", "7", "3"),
            ("upow", "2", "3"),
        ]
    };
    for (name, left, right) in arith {
        calls.push((name.to_owned(), vec![param(left), param(right)], None));
    }
    let traps: Vec<(&str, &str, &str)> = if big {
        vec![
            ("uadd", "18446744073709551615", "1"),
            ("usub", "0", "1"),
            ("umul", "18446744073709551615", "2"),
            ("udiv", "7", "0"),
            ("upow", "2", "64"),
        ]
    } else {
        vec![
            ("uadd", "4294967295", "1"),
            ("usub", "0", "1"),
            ("umul", "100000", "100000"),
            ("udiv", "7", "0"),
            ("upow", "10", "10"),
        ]
    };
    for (name, left, right) in traps {
        calls.push((
            format!("{name}_trap"),
            vec![name.to_owned(), param(left), param(right)],
            None,
        ));
    }
    // Above-signed-maximum comparisons, mixed-sign comparisons and the infallible
    // Uint-to-Float conversions (bit-exact F32/F64 rendering).
    if big {
        calls.push((
            "ucmp_lt".to_owned(),
            vec![param("9223372036854775807"), param("9223372036854775808")],
            None,
        ));
        calls.push((
            "ucmp_ge".to_owned(),
            vec![param("9223372036854775808"), param("9223372036854775808")],
            None,
        ));
        calls.push((
            "ucmp_eq".to_owned(),
            vec![param("18446744073709551615"), param("18446744073709551615")],
            None,
        ));
        calls.push(("umix_lt".to_owned(), vec![param("-1"), param("0")], None));
        calls.push((
            "umix_gt".to_owned(),
            vec![param("9223372036854775808"), param("9223372036854775807")],
            None,
        ));
    } else {
        calls.push((
            "ucmp_lt".to_owned(),
            vec![param("2147483647"), param("2147483648")],
            None,
        ));
        calls.push((
            "ucmp_ge".to_owned(),
            vec![param("4294967295"), param("4294967295")],
            None,
        ));
        calls.push((
            "ucmp_eq".to_owned(),
            vec![param("4294967295"), param("4294967295")],
            None,
        ));
        calls.push(("umix_lt".to_owned(), vec![param("-1"), param("0")], None));
        calls.push((
            "umix_gt".to_owned(),
            vec![param("2147483648"), param("2147483647")],
            None,
        ));
    }
    calls.push(("uto_float".to_owned(), vec![], Some("f64".to_owned())));
    if big {
        calls.push(("uto_f32".to_owned(), vec![], Some("f32".to_owned())));
    }
    calls
}

fn uint_scalar_expectations(profile: NumericProfile) -> Vec<(String, String, String)> {
    // I64-carrier results render through the signed JS BigInt view, so values above the signed
    // maximum observe as their signed reinterpretation: the assertions pin exact bit patterns.
    let mut expected = if profile.int_width == IntWidth::Bits64 {
        vec![
            (
                "uadd".to_owned(),
                "ok".to_owned(),
                "-9223372036854775708".to_owned(),
            ),
            (
                "usub".to_owned(),
                "ok".to_owned(),
                "9223372036854775807".to_owned(),
            ),
            ("umul".to_owned(), "ok".to_owned(), "-709551616".to_owned()),
            ("udiv".to_owned(), "ok".to_owned(), "1".to_owned()),
            (
                "urem".to_owned(),
                "ok".to_owned(),
                "9223372036854775807".to_owned(),
            ),
            ("upow".to_owned(), "ok".to_owned(), "1024".to_owned()),
        ]
    } else {
        // I32-carrier results render through the signed JS Number view, so the unsigned
        // maximum 4294967295 observes as -1: the assertion is on the exact bit pattern.
        vec![
            ("uadd".to_owned(), "ok".to_owned(), "-294967295".to_owned()),
            ("usub".to_owned(), "ok".to_owned(), "2".to_owned()),
            ("umul".to_owned(), "ok".to_owned(), "-1".to_owned()),
            ("udiv".to_owned(), "ok".to_owned(), "3".to_owned()),
            ("urem".to_owned(), "ok".to_owned(), "1".to_owned()),
            ("upow".to_owned(), "ok".to_owned(), "8".to_owned()),
        ]
    };
    for name in ["uadd", "usub", "umul", "udiv", "upow"] {
        expected.push((
            format!("{name}_trap"),
            "trap".to_owned(),
            "unreachable".to_owned(),
        ));
    }
    expected.push(("ucmp_lt".to_owned(), "ok".to_owned(), "1".to_owned()));
    expected.push(("ucmp_ge".to_owned(), "ok".to_owned(), "1".to_owned()));
    expected.push(("ucmp_eq".to_owned(), "ok".to_owned(), "1".to_owned()));
    expected.push(("umix_lt".to_owned(), "ok".to_owned(), "1".to_owned()));
    expected.push(("umix_gt".to_owned(), "ok".to_owned(), "1".to_owned()));
    // 3000000001 is exactly representable in F64; the Float32 witness input rounds directly
    // to 9007200000000000 (F32 bits 0x5a000001).
    expected.push((
        "uto_float".to_owned(),
        "ok".to_owned(),
        "41e65a0bc0200000".to_owned(),
    ));
    if profile.int_width == IntWidth::Bits64 {
        expected.push(("uto_f32".to_owned(), "ok".to_owned(), "5a000001".to_owned()));
    }
    expected
}

/// Executes named Uint exports in Node, distinguishing terminal Wasm traps from returned values.
fn execute_wasm_uint_exports_in_node(
    wasm_bytes: &[u8],
    calls: &[(String, Vec<String>, Option<String>)],
) -> Vec<(String, String, String)> {
    const NODE_BODY: &str = r#"
const results = [];
for (const [name, args, float] of __CALLS__) {
  const callName = args.length > 0 && typeof args[0] === "string" ? args[0] : name;
  const callArgs = args.length > 0 && typeof args[0] === "string" ? args.slice(1) : args;
  try {
    const value = instance.exports[callName](...callArgs);
    let rendered = String(value);
    if (float === "f64") {
      const view = new DataView(new ArrayBuffer(8));
      view.setFloat64(0, value, true);
      rendered = view.getBigUint64(0, true).toString(16).padStart(16, "0");
    } else if (float === "f32") {
      const view = new DataView(new ArrayBuffer(4));
      view.setFloat32(0, value, true);
      rendered = view.getUint32(0, true).toString(16).padStart(8, "0");
    }
    results.push(`${name}|ok|${rendered}`);
  } catch (error) {
    if (error !== null
        && typeof error === "object"
        && Object.getPrototypeOf(error) === WebAssembly.RuntimeError.prototype) {
      results.push(`${name}|trap|${String(error.message)}`);
    } else {
      throw error;
    }
  }
}
process.stdout.write(results.join("\n"));
"#;
    let calls = calls
        .iter()
        .map(|(name, args, float)| {
            let float = match float.as_deref() {
                Some("f64") => "\"f64\"",
                Some("f32") => "\"f32\"",
                None => "null",
                Some(other) => panic!("unsupported Uint float rendering {other}"),
            };
            let rendered = args
                .iter()
                .map(|argument| {
                    if argument.ends_with('n')
                        && argument[..argument.len() - 1].parse::<i128>().is_ok()
                    {
                        argument.trim_end_matches('n').to_owned() + "n"
                    } else if argument.parse::<i128>().is_ok() {
                        argument.clone()
                    } else {
                        format!("{argument:?}")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{name:?}, [{rendered}], {float}]")
        })
        .collect::<Vec<_>>()
        .join(", ");
    let node_body = NODE_BODY.replace("__CALLS__", &format!("[{calls}]"));
    let output = run_wasm_node_script(wasm_bytes, "", &node_body);
    String::from_utf8(output)
        .expect("Node Uint results should be UTF-8")
        .lines()
        .map(|line| {
            let mut parts = line.splitn(3, '|');
            (
                parts
                    .next()
                    .expect("result should include export name")
                    .to_owned(),
                parts
                    .next()
                    .expect("result should include outcome")
                    .to_owned(),
                parts
                    .next()
                    .expect("result should include value or trap")
                    .to_owned(),
            )
        })
        .collect()
}
