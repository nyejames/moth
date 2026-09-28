use crate::backends::error_types::BackendErrorType;
use crate::backends::wasm::backend::lower_hir_to_wasm_module;
use crate::backends::wasm::emit::module::emit_lir_to_wasm_module;
use crate::backends::wasm::lir::function::{WasmLirBlock, WasmLirFunction, WasmLirFunctionOrigin};
use crate::backends::wasm::lir::instructions::{
    WasmCalleeRef, WasmIntegerOperationKind, WasmIntegerOperationOperands, WasmIntegerPowerScratch,
    WasmIntegerScratch, WasmLirStmt, WasmLirTerminator,
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
use crate::backends::wasm::tests::lowering::test_support::{
    build_module, build_type_environment, default_borrow_facts, int_expression,
};
use crate::compiler_frontend::compiler_messages::compiler_errors::ErrorType;
use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_profile::{
    FloatPrecision, IntWidth, NumericProfile,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, RegionId};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
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
        &request,
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("Wasm lowering should emit module bytes");
    let wasm_bytes = result.wasm_bytes.expect("wasm bytes should be available");
    validate_wasm(&wasm_bytes);
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
            dst: WasmLirLocalId(13),
            operator: NumericOperator::Add,
            kind: WasmIntegerOperationKind::Signed64,
            operands: WasmIntegerOperationOperands::Binary {
                left: WasmLirLocalId(1),
                right: WasmLirLocalId(3),
            },
            scratch: WasmIntegerScratch {
                product: None,
                power: None,
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
    let actual = execute_wasm_checked_exports_in_node(&result.wasm_bytes, &case_names);
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
fn scalar_storage_facts_follow_profile_and_natural_layout() {
    let kinds = [
        (WasmScalarStorageKind::I8, 1, WasmAbiType::I32),
        (WasmScalarStorageKind::U8, 1, WasmAbiType::I32),
        (WasmScalarStorageKind::I16, 2, WasmAbiType::I32),
        (WasmScalarStorageKind::U16, 2, WasmAbiType::I32),
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
    }
    assert_eq!(
        WasmScalarStorageKind::from_fixed_scalar(FixedScalar::Byte),
        Some(WasmScalarStorageKind::U8)
    );
    assert_eq!(
        WasmScalarStorageKind::from_fixed_scalar(FixedScalar::F16),
        None
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
    let mut expected = vec![0u8; 120];
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
    assert_eq!(actual, expected);
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
                    dst: WasmLirLocalId(13),
                    operator: NumericOperator::Add,
                    kind: WasmIntegerOperationKind::Signed64,
                    operands: WasmIntegerOperationOperands::Binary {
                        left: WasmLirLocalId(1),
                        right: WasmLirLocalId(1),
                    },
                    scratch: WasmIntegerScratch {
                        product: None,
                        power: None,
                    },
                },
                WasmLirStmt::FloatAdd {
                    dst: WasmLirLocalId(14),
                    lhs: WasmLirLocalId(3),
                    rhs: WasmLirLocalId(3),
                },
                WasmLirStmt::CheckedIntegerOp {
                    dst: WasmLirLocalId(20),
                    operator: NumericOperator::Subtract,
                    kind: WasmIntegerOperationKind::Signed64,
                    operands: WasmIntegerOperationOperands::Binary {
                        left: WasmLirLocalId(13),
                        right: WasmLirLocalId(1),
                    },
                    scratch: WasmIntegerScratch {
                        product: None,
                        power: None,
                    },
                },
                WasmLirStmt::FloatSub {
                    dst: WasmLirLocalId(21),
                    lhs: WasmLirLocalId(14),
                    rhs: WasmLirLocalId(3),
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
        dst: operation_result,
        operator,
        kind,
        operands: WasmIntegerOperationOperands::Binary { left, right },
        scratch: WasmIntegerScratch {
            product: product_scratch,
            power: power_scratch,
        },
    });

    let return_local = if let Some(sum) = source_sum {
        statements.push(WasmLirStmt::CheckedIntegerOp {
            dst: sum,
            operator: NumericOperator::Add,
            kind,
            operands: WasmIntegerOperationOperands::Binary { left, right },
            scratch: WasmIntegerScratch {
                product: None,
                power: None,
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

fn execute_wasm_checked_exports_in_node(
    wasm_bytes: &[u8],
    export_names: &[&str],
) -> Vec<(String, String, String)> {
    const NODE_BODY: &str = r#"
const results = [];
for (const name of __EXPORT_NAMES__) {
  try {
    results.push(`${name}|ok|${String(instance.exports[name]())}`);
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
    let export_names = export_names
        .iter()
        .map(|name| format!("{name:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let node_body = NODE_BODY.replace("__EXPORT_NAMES__", &format!("[{export_names}]"));
    let output = run_wasm_node_script(wasm_bytes, &node_body);
    String::from_utf8(output)
        .expect("Node checked integer results should be UTF-8")
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
process.stdout.write(Buffer.from(instance.exports.memory.buffer, 0, 120));
"#;
    run_wasm_node_script(wasm_bytes, NODE_BODY)
}

fn run_wasm_node_script(wasm_bytes: &[u8], node_body: &str) -> Vec<u8> {
    const NODE_SCRIPT: &str = r#"
const chunks = [];
process.stdin.on("data", chunk => chunks.push(chunk));
process.stdin.on("end", async () => {
  try {
    const { instance } = await WebAssembly.instantiate(Buffer.concat(chunks));
    __NODE_BODY__
  } catch (error) {
    console.error(error && error.stack ? error.stack : error);
    process.exitCode = 1;
  }
});
"#;
    let node_script = NODE_SCRIPT.replace("__NODE_BODY__", node_body);
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
