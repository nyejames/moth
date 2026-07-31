use crate::backends::error_types::BackendErrorType;
use crate::backends::wasm::backend::lower_hir_to_wasm_module;
use crate::backends::wasm::emit::module::emit_lir_to_wasm_module;
use crate::backends::wasm::lir::function::{WasmLirBlock, WasmLirFunction, WasmLirFunctionOrigin};
use crate::backends::wasm::lir::instructions::{WasmCalleeRef, WasmLirStmt, WasmLirTerminator};
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
use crate::backends::wasm::runtime::memory::WasmMemoryPlan;
use crate::backends::wasm::tests::lowering::test_support::{
    build_module, build_type_environment, default_borrow_facts, int_expression,
};
use crate::compiler_frontend::compiler_messages::compiler_errors::ErrorType;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, RegionId};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::FxHashMap;

#[test]
fn lowers_hir_to_wasm_module_bytes() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let start_path = InternedPath::from_single_str("main", &mut string_table);

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
    let (type_environment, types) = build_type_environment();
    let start_path = InternedPath::from_single_str("main", &mut string_table);
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
    )
    .expect_err("invalid helper policy should fail");
    let (error_type, message, _location) = error
        .first_infrastructure_error_for_tests()
        .expect("Wasm generation failure should be wrapped for rendering");
    assert_eq!(
        error_type,
        &ErrorType::Backend(BackendErrorType::WasmGeneration)
    );
    assert!(message.contains("moth_str_ptr"));
}

#[test]
fn rejects_mismatched_numeric_add_types() {
    let mut module = build_manual_lir_module();
    let main = module
        .functions
        .iter_mut()
        .find(|function| function.id == WasmLirFunctionId(1))
        .expect("manual main function should be present");

    main.blocks[0].statements.push(WasmLirStmt::IntAdd {
        dst: WasmLirLocalId(13),
        lhs: WasmLirLocalId(1),
        rhs: WasmLirLocalId(3),
    });

    let error = emit_lir_to_wasm_module(&module, &WasmBackendRequest::default())
        .expect_err("mismatched IntAdd operands should fail emission");
    assert_eq!(
        error.error_type,
        ErrorType::Backend(BackendErrorType::WasmGeneration)
    );
    assert!(error.msg.contains("type mismatch in numeric add"));
}

#[test]
fn rejects_unsupported_wasm_feature_flags() {
    let mut request = WasmBackendRequest::default();
    request.target_features.use_wasm_gc = true;

    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let start_path = InternedPath::from_single_str("main", &mut string_table);
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
    )
    .expect_err("unsupported feature toggle should fail");
    let (error_type, message, _location) = error
        .first_infrastructure_error_for_tests()
        .expect("Wasm generation failure should be wrapped for rendering");
    assert_eq!(
        error_type,
        &ErrorType::Backend(BackendErrorType::WasmGeneration)
    );
    assert!(message.contains("use_wasm_gc"));
}

#[test]
fn rejects_unsupported_cfg_lowering_strategy() {
    let mut request = WasmBackendRequest::default();
    request.emit_options.cfg_lowering_strategy = WasmCfgLoweringStrategy::Structured;

    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let start_path = InternedPath::from_single_str("main", &mut string_table);
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
    )
    .expect_err("unsupported cfg strategy should fail");
    let (error_type, message, _location) = error
        .first_infrastructure_error_for_tests()
        .expect("Wasm generation failure should be wrapped for rendering");
    assert_eq!(
        error_type,
        &ErrorType::Backend(BackendErrorType::WasmGeneration)
    );
    assert!(message.contains("dispatcher-loop"));
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
        function_emission_policy: Default::default(),
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
                WasmLirStmt::IntAdd {
                    dst: WasmLirLocalId(13),
                    lhs: WasmLirLocalId(1),
                    rhs: WasmLirLocalId(1),
                },
                WasmLirStmt::FloatAdd {
                    dst: WasmLirLocalId(14),
                    lhs: WasmLirLocalId(3),
                    rhs: WasmLirLocalId(3),
                },
                WasmLirStmt::IntSub {
                    dst: WasmLirLocalId(20),
                    lhs: WasmLirLocalId(13),
                    rhs: WasmLirLocalId(1),
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
