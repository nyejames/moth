use crate::backends::wasm::backend::lower_hir_to_wasm_lir;
use crate::backends::wasm::hir_to_lir::context::lower_type_to_abi;
use crate::backends::wasm::lir::function::WasmLirFunctionOrigin;
use crate::backends::wasm::lir::instructions::{WasmCalleeRef, WasmLirStmt, WasmLirTerminator};
use crate::backends::wasm::lir::linkage::{WasmExportKind, WasmFunctionLinkage};
use crate::backends::wasm::lir::types::{WasmAbiType, WasmLirFunctionId, WasmLirLocalId};
use crate::backends::wasm::request::{
    WasmBackendRequest, WasmDebugFlags, WasmExportPolicy, WasmFunctionEmissionPolicy,
};
use crate::backends::wasm::tests::lowering::test_support::{
    bool_expression, borrow_facts_with_drop_site, build_module, build_type_environment,
    default_borrow_facts, expression, int_expression, load_local, local, statement,
    string_expression, unit_expression,
};
use crate::compiler_frontend::analysis::borrow_checker::BorrowDropSiteKind;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};

use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::operators::HirBinOp;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::FxHashMap;

#[test]
fn lowers_calls_and_cfg_with_resolvable_branch_targets() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let callee_path = InternedPath::from_single_str("callee", &mut string_table);
    let main_path = InternedPath::from_single_str("main", &mut string_table);

    let callee_block = HirBlock {
        id: BlockId(10),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(100, 7, types.int, RegionId(0))),
    };

    let main_entry = HirBlock {
        id: BlockId(30),
        region: RegionId(0),
        locals: vec![
            local(0, types.boolean, RegionId(0)),
            local(1, types.int, RegionId(0)),
        ],
        statements: vec![
            statement(
                1,
                HirStatementKind::Assign {
                    target: HirPlace::Local(LocalId(0)),
                    value: bool_expression(101, true, types.boolean, RegionId(0)),
                },
                1,
            ),
            statement(
                2,
                HirStatementKind::Call {
                    target: CallTarget::UserFunction(FunctionId(0)),
                    args: vec![],
                    result: Some(LocalId(1)),
                },
                2,
            ),
        ],
        terminator: HirTerminator::If {
            condition: load_local(102, LocalId(0), types.boolean, RegionId(0)),
            then_block: BlockId(40),
            else_block: BlockId(50),
        },
    };

    let then_block = HirBlock {
        id: BlockId(40),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(load_local(103, LocalId(1), types.int, RegionId(0))),
    };

    let else_block = HirBlock {
        id: BlockId(50),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(104, 0, types.int, RegionId(0))),
    };

    let callee = HirFunction {
        id: FunctionId(0),
        entry: BlockId(10),
        params: vec![],
        return_type: types.int,
        return_aliases: vec![],
    };
    let main = HirFunction {
        id: FunctionId(1),
        entry: BlockId(30),
        params: vec![],
        return_type: types.int,
        return_aliases: vec![],
    };

    let module = build_module(
        &mut string_table,
        vec![
            (callee, callee_path, HirFunctionOrigin::Normal),
            (main, main_path, HirFunctionOrigin::EntryStart),
        ],
        vec![callee_block, main_entry, then_block, else_block],
        FunctionId(1),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
    )
    .expect("Wasm lowering should succeed");

    let main_lir = result
        .lir_module
        .functions
        .iter()
        .find(|function| function.id == WasmLirFunctionId(1))
        .expect("lowered main function should be present");
    assert_eq!(main_lir.blocks.len(), 3);

    // WHAT: the lowering renumbers reachable blocks into a complete, unique LIR block set.
    // WHY: emission re-indexes blocks by sorted id, so the exact sequential LIR block ids are not
    // a stable contract. Assert a complete unique mapping with resolvable branch targets instead
    // of pinning the incidental sequential renumbering.
    let mut block_ids: Vec<u32> = main_lir.blocks.iter().map(|block| block.id.0).collect();
    block_ids.sort_unstable();
    block_ids.dedup();
    assert_eq!(block_ids.len(), 3, "lowered blocks should have unique ids");

    let entry_block = main_lir
        .blocks
        .iter()
        .find(|block| {
            block.statements.iter().any(|statement| {
                matches!(
                    statement,
                    WasmLirStmt::Call {
                        callee: WasmCalleeRef::Function(WasmLirFunctionId(0)),
                        ..
                    }
                )
            })
        })
        .expect("entry block should contain the lowered callee invocation");

    // The branch targets are LIR block ids produced by the internal renumbering; assert they
    // resolve to real lowered blocks rather than pinning the incidental sequential values.
    let WasmLirTerminator::Branch {
        then_block,
        else_block,
        ..
    } = &entry_block.terminator
    else {
        panic!("entry block terminator should lower to a Branch");
    };
    assert_ne!(then_block, else_block, "branch targets should be distinct");
    assert!(
        main_lir.blocks.iter().any(|block| block.id == *then_block),
        "then target should resolve to a lowered block"
    );
    assert!(
        main_lir.blocks.iter().any(|block| block.id == *else_block),
        "else target should resolve to a lowered block"
    );
}

#[test]
fn lowers_runtime_template_with_literal_and_handle_chunks_in_order() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let runtime_path = InternedPath::from_single_str("__moth_frag_0", &mut string_table);

    let concat = expression(
        202,
        HirExpressionKind::BinOp {
            left: Box::new(expression(
                201,
                HirExpressionKind::BinOp {
                    left: Box::new(string_expression(200, "a", types.string, RegionId(0))),
                    op: HirBinOp::Add,
                    right: Box::new(load_local(203, LocalId(0), types.string, RegionId(0))),
                },
                types.string,
                RegionId(0),
                ValueKind::RValue,
            )),
            op: HirBinOp::Add,
            right: Box::new(string_expression(204, "b", types.string, RegionId(0))),
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
    );

    let runtime_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.string, RegionId(0))],
        statements: vec![],
        terminator: HirTerminator::Return(concat),
    };

    let runtime_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![LocalId(0)],
        return_type: types.string,
        return_aliases: vec![],
    };

    let module = build_module(
        &mut string_table,
        vec![(runtime_function, runtime_path, HirFunctionOrigin::Normal)],
        vec![runtime_block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
    )
    .expect("Wasm lowering should succeed");

    let runtime_lir = result
        .lir_module
        .functions
        .iter()
        .find(|function| function.id == WasmLirFunctionId(0))
        .expect("lowered runtime function should be present");
    let statements = &runtime_lir.blocks[0].statements;

    assert!(matches!(statements[0], WasmLirStmt::StringNewBuffer { .. }));
    assert!(matches!(
        statements[1],
        WasmLirStmt::StringPushLiteral { .. }
    ));
    assert!(matches!(
        statements[2],
        WasmLirStmt::StringPushHandle { .. }
    ));
    assert!(matches!(
        statements[3],
        WasmLirStmt::StringPushLiteral { .. }
    ));
    assert!(matches!(statements[4], WasmLirStmt::StringFinish { .. }));
}

#[test]
fn lowers_runtime_template_with_cfg_before_final_return() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let runtime_path = InternedPath::from_single_str("__moth_frag_cfg", &mut string_table);

    let entry_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![statement(
            300,
            HirStatementKind::Assign {
                target: HirPlace::Local(LocalId(0)),
                value: int_expression(301, 0, types.int, RegionId(0)),
            },
            300,
        )],
        terminator: HirTerminator::Jump {
            target: BlockId(1),
            args: vec![],
        },
    };

    let header_block = HirBlock {
        id: BlockId(1),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::If {
            condition: expression(
                302,
                HirExpressionKind::BinOp {
                    left: Box::new(load_local(303, LocalId(0), types.int, RegionId(0))),
                    op: HirBinOp::Lt,
                    right: Box::new(int_expression(304, 2, types.int, RegionId(0))),
                },
                types.boolean,
                RegionId(0),
                ValueKind::RValue,
            ),
            then_block: BlockId(2),
            else_block: BlockId(3),
        },
    };

    let body_block = HirBlock {
        id: BlockId(2),
        region: RegionId(0),
        locals: vec![],
        statements: vec![statement(
            305,
            HirStatementKind::Assign {
                target: HirPlace::Local(LocalId(0)),
                value: expression(
                    306,
                    HirExpressionKind::BinOp {
                        left: Box::new(load_local(307, LocalId(0), types.int, RegionId(0))),
                        op: HirBinOp::Add,
                        right: Box::new(int_expression(308, 1, types.int, RegionId(0))),
                    },
                    types.int,
                    RegionId(0),
                    ValueKind::RValue,
                ),
            },
            305,
        )],
        terminator: HirTerminator::Jump {
            target: BlockId(1),
            args: vec![],
        },
    };

    let exit_block = HirBlock {
        id: BlockId(3),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(string_expression(
            309,
            "runtime loop done",
            types.string,
            RegionId(0),
        )),
    };

    let runtime_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.string,
        return_aliases: vec![],
    };

    let module = build_module(
        &mut string_table,
        vec![(runtime_function, runtime_path, HirFunctionOrigin::Normal)],
        vec![entry_block, header_block, body_block, exit_block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
    )
    .expect("runtime template CFG should lower successfully");

    let runtime_lir = result
        .lir_module
        .functions
        .iter()
        .find(|function| function.id == WasmLirFunctionId(0))
        .expect("runtime CFG function should be present");

    assert_eq!(runtime_lir.blocks.len(), 4);
    assert!(matches!(
        runtime_lir.blocks[0].terminator,
        WasmLirTerminator::Jump(_)
    ));
    assert!(matches!(
        runtime_lir.blocks[1].terminator,
        WasmLirTerminator::Branch { .. }
    ));
    assert!(matches!(
        runtime_lir.blocks[3].terminator,
        WasmLirTerminator::Return { value: Some(_) }
    ));
}

#[test]
fn lowers_non_runtime_string_add_as_buffer_concat() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let function_path = InternedPath::from_single_str("render_title", &mut string_table);

    let concat = expression(
        1300,
        HirExpressionKind::BinOp {
            left: Box::new(string_expression(
                1301,
                "Title: ",
                types.string,
                RegionId(0),
            )),
            op: HirBinOp::Add,
            right: Box::new(load_local(1302, LocalId(0), types.string, RegionId(0))),
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.string, RegionId(0))],
        statements: vec![],
        terminator: HirTerminator::Return(concat),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![LocalId(0)],
        return_type: types.string,
        return_aliases: vec![],
    };
    let module = build_module(
        &mut string_table,
        vec![(function, function_path, HirFunctionOrigin::Normal)],
        vec![block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
    )
    .expect("non-runtime string Add should lower to buffer operations");

    let lowered = result
        .lir_module
        .functions
        .iter()
        .find(|function| function.id == WasmLirFunctionId(0))
        .expect("lowered function should be present");
    let statements = &lowered.blocks[0].statements;

    assert!(matches!(statements[0], WasmLirStmt::StringNewBuffer { .. }));
    assert!(matches!(
        statements[1],
        WasmLirStmt::StringPushLiteral { .. }
    ));
    assert!(matches!(
        statements[2],
        WasmLirStmt::StringPushHandle { .. }
    ));
    assert!(matches!(statements[3], WasmLirStmt::StringFinish { .. }));
}

#[test]
fn lowers_non_runtime_string_add_with_i64_chunk_via_string_from_i64() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let function_path = InternedPath::from_single_str("render_runtime_int", &mut string_table);

    let concat = expression(
        1310,
        HirExpressionKind::BinOp {
            left: Box::new(string_expression(1311, "", types.string, RegionId(0))),
            op: HirBinOp::Add,
            right: Box::new(load_local(1312, LocalId(0), types.int, RegionId(0))),
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
    );
    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![],
        terminator: HirTerminator::Return(concat),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![LocalId(0)],
        return_type: types.string,
        return_aliases: vec![],
    };
    let module = build_module(
        &mut string_table,
        vec![(function, function_path, HirFunctionOrigin::Normal)],
        vec![block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
    )
    .expect("non-runtime string Add should bridge i64 chunks");
    let lowered = result
        .lir_module
        .functions
        .iter()
        .find(|function| function.id == WasmLirFunctionId(0))
        .expect("lowered function should be present");

    assert!(
        lowered.blocks[0]
            .statements
            .iter()
            .any(|statement| matches!(statement, WasmLirStmt::StringFromI64 { .. })),
        "lowered statements should include i64-to-string bridge"
    );
}

#[test]
fn lowers_ordered_comparison_and_numeric_add_for_control_flow() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let function_path = InternedPath::from_single_str("loop_like_fn", &mut string_table);

    let condition = expression(
        1400,
        HirExpressionKind::BinOp {
            left: Box::new(load_local(1401, LocalId(0), types.int, RegionId(0))),
            op: HirBinOp::Le,
            right: Box::new(int_expression(1402, 5, types.int, RegionId(0))),
        },
        types.boolean,
        RegionId(0),
        ValueKind::RValue,
    );
    let updated_sum = expression(
        1403,
        HirExpressionKind::BinOp {
            left: Box::new(load_local(1404, LocalId(1), types.int, RegionId(0))),
            op: HirBinOp::Add,
            right: Box::new(int_expression(1405, 1, types.int, RegionId(0))),
        },
        types.int,
        RegionId(0),
        ValueKind::RValue,
    );
    let decremented_counter = expression(
        1410,
        HirExpressionKind::BinOp {
            left: Box::new(load_local(1411, LocalId(0), types.int, RegionId(0))),
            op: HirBinOp::Sub,
            right: Box::new(int_expression(1412, 1, types.int, RegionId(0))),
        },
        types.int,
        RegionId(0),
        ValueKind::RValue,
    );

    let entry_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![
            local(0, types.int, RegionId(0)),
            local(1, types.int, RegionId(0)),
        ],
        statements: vec![
            statement(
                1,
                HirStatementKind::Assign {
                    target: HirPlace::Local(LocalId(0)),
                    value: int_expression(1406, 0, types.int, RegionId(0)),
                },
                1,
            ),
            statement(
                2,
                HirStatementKind::Assign {
                    target: HirPlace::Local(LocalId(1)),
                    value: int_expression(1407, 10, types.int, RegionId(0)),
                },
                2,
            ),
        ],
        terminator: HirTerminator::If {
            condition,
            then_block: BlockId(1),
            else_block: BlockId(2),
        },
    };
    let then_block = HirBlock {
        id: BlockId(1),
        region: RegionId(0),
        locals: vec![],
        statements: vec![
            statement(
                3,
                HirStatementKind::Assign {
                    target: HirPlace::Local(LocalId(1)),
                    value: updated_sum,
                },
                3,
            ),
            statement(
                4,
                HirStatementKind::Assign {
                    target: HirPlace::Local(LocalId(0)),
                    value: decremented_counter,
                },
                4,
            ),
        ],
        terminator: HirTerminator::Return(load_local(1408, LocalId(1), types.int, RegionId(0))),
    };
    let else_block = HirBlock {
        id: BlockId(2),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(1409, 0, types.int, RegionId(0))),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.int,
        return_aliases: vec![],
    };
    let module = build_module(
        &mut string_table,
        vec![(function, function_path, HirFunctionOrigin::Normal)],
        vec![entry_block, then_block, else_block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
    )
    .expect("ordered comparisons and Add should lower in non-runtime functions");

    let lowered = result
        .lir_module
        .functions
        .iter()
        .find(|function| function.id == WasmLirFunctionId(0))
        .expect("lowered function should be present");

    assert!(
        lowered.blocks[0]
            .statements
            .iter()
            .any(|statement| matches!(statement, WasmLirStmt::OrderedLe { .. })),
        "entry block should include ordered comparison lowering"
    );
    assert!(
        lowered.blocks[1]
            .statements
            .iter()
            .any(|statement| matches!(statement, WasmLirStmt::IntAdd { .. })),
        "then block should include numeric add lowering"
    );
    assert!(
        lowered.blocks[1]
            .statements
            .iter()
            .any(|statement| matches!(statement, WasmLirStmt::IntSub { .. })),
        "then block should include numeric sub lowering"
    );
}

#[test]
fn deduplicates_static_utf8_segments() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let start_path = InternedPath::from_single_str("main", &mut string_table);

    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![
            local(0, types.string, RegionId(0)),
            local(1, types.string, RegionId(0)),
        ],
        statements: vec![
            statement(
                1,
                HirStatementKind::Assign {
                    target: HirPlace::Local(LocalId(0)),
                    value: string_expression(300, "same", types.string, RegionId(0)),
                },
                1,
            ),
            statement(
                2,
                HirStatementKind::Assign {
                    target: HirPlace::Local(LocalId(1)),
                    value: string_expression(301, "same", types.string, RegionId(0)),
                },
                2,
            ),
        ],
        terminator: HirTerminator::Return(unit_expression(302, types.unit, RegionId(0))),
    };

    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
        return_aliases: vec![],
    };

    let module = build_module(
        &mut string_table,
        vec![(start_function, start_path, HirFunctionOrigin::EntryStart)],
        vec![start_block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
    )
    .expect("Wasm lowering should succeed");
    assert_eq!(result.lir_module.static_data.len(), 1);
}

#[test]
fn maps_advisory_drop_sites_to_drop_if_owned_statements() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let start_path = InternedPath::from_single_str("main", &mut string_table);

    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.string, RegionId(0))],
        statements: vec![],
        terminator: HirTerminator::Return(unit_expression(401, types.unit, RegionId(0))),
    };
    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
        return_aliases: vec![],
    };
    let module = build_module(
        &mut string_table,
        vec![(start_function, start_path, HirFunctionOrigin::EntryStart)],
        vec![start_block],
        FunctionId(0),
    );

    let borrow_facts =
        borrow_facts_with_drop_site(BlockId(0), BorrowDropSiteKind::Return, vec![LocalId(0)]);

    let result = lower_hir_to_wasm_lir(
        &module,
        &borrow_facts,
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
    )
    .expect("Wasm lowering should succeed");
    let lowered_start = result
        .lir_module
        .functions
        .iter()
        .find(|function| function.id == WasmLirFunctionId(0))
        .expect("lowered start function should be present");
    // WHAT: the advisory drop site must lower to a DropIfOwned on the owned string handle.
    // WHY: LIR local ids are an internal renumbering that emission re-indexes, so identify the
    // drop target by its handle ABI type rather than the incidental sequential local id.
    let handle_locals: Vec<WasmLirLocalId> = lowered_start
        .locals
        .iter()
        .filter(|local| local.ty == WasmAbiType::Handle)
        .map(|local| local.id)
        .collect();
    assert_eq!(
        handle_locals.len(),
        1,
        "fixture should declare exactly one owned handle local"
    );
    assert!(lowered_start.blocks[0].statements.iter().any(
        |statement| matches!(statement, WasmLirStmt::DropIfOwned { value } if *value == handle_locals[0])
    ));
}

#[test]
fn synthesizes_export_wrappers_with_stable_names() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let start_path = InternedPath::from_single_str("main", &mut string_table);

    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(500, 123, types.int, RegionId(0))),
    };
    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.int,
        return_aliases: vec![],
    };
    let module = build_module(
        &mut string_table,
        vec![(start_function, start_path, HirFunctionOrigin::EntryStart)],
        vec![start_block],
        FunctionId(0),
    );

    let mut export_names = FxHashMap::default();
    export_names.insert(FunctionId(0), "main".to_owned());

    let request = WasmBackendRequest {
        export_policy: WasmExportPolicy {
            exported_functions: vec![FunctionId(0)],
            export_names,
            helper_exports: Default::default(),
        },
        target_features: Default::default(),
        emit_options: Default::default(),
        debug_flags: WasmDebugFlags {
            show_wasm_exports: true,
            ..Default::default()
        },
        external_package_registry: Default::default(),
        function_emission_policy: Default::default(),
    };

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &request,
        &string_table,
        &type_environment,
    )
    .expect("Wasm lowering should succeed");

    assert_eq!(result.lir_module.exports.len(), 1);
    let WasmExportKind::Function(wrapper_id) = result.lir_module.exports[0].kind;
    assert_eq!(result.lir_module.exports[0].export_name, "main");

    let wrapper = result
        .lir_module
        .functions
        .iter()
        .find(|function| function.id == wrapper_id)
        .expect("wrapper function should be present");
    assert!(matches!(
        wrapper.origin,
        WasmLirFunctionOrigin::ExportWrapper
    ));
    assert!(matches!(
        wrapper.linkage,
        WasmFunctionLinkage::ExportedWrapper
    ));
    assert!(
        wrapper.blocks[0]
            .statements
            .iter()
            .any(|statement| matches!(
                statement,
                WasmLirStmt::Call {
                    callee: WasmCalleeRef::Function(WasmLirFunctionId(0)),
                    ..
                }
            ))
    );
}

#[test]
fn rejects_invalid_export_request_with_structured_diagnostic() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let start_path = InternedPath::from_single_str("main", &mut string_table);

    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(600, 1, types.int, RegionId(0))),
    };
    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.int,
        return_aliases: vec![],
    };
    let module = build_module(
        &mut string_table,
        vec![(start_function, start_path, HirFunctionOrigin::EntryStart)],
        vec![start_block],
        FunctionId(0),
    );

    let invalid_request = WasmBackendRequest {
        export_policy: WasmExportPolicy {
            exported_functions: vec![FunctionId(0)],
            export_names: FxHashMap::default(),
            helper_exports: Default::default(),
        },
        ..Default::default()
    };

    let error = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &invalid_request,
        &string_table,
        &type_environment,
    )
    .expect_err("invalid request should produce a lowering diagnostic");
    let (_error_type, message, _location) = error
        .first_infrastructure_error_for_tests()
        .expect("Wasm lowering failure should be wrapped for rendering");
    assert!(message.contains("missing stable export name for FunctionId(0)"));
}

#[test]
fn rejects_unsupported_host_call_with_diagnostic() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let start_path = InternedPath::from_single_str("main", &mut string_table);

    let unknown_id =
        crate::compiler_frontend::external_packages::ExternalFunctionId::Synthetic(9999);
    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![statement(
            1,
            HirStatementKind::Call {
                target: CallTarget::ExternalFunction(unknown_id),
                args: vec![],
                result: None,
            },
            1,
        )],
        terminator: HirTerminator::Return(unit_expression(800, types.unit, RegionId(0))),
    };

    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
        return_aliases: vec![],
    };
    let module = build_module(
        &mut string_table,
        vec![(start_function, start_path, HirFunctionOrigin::EntryStart)],
        vec![start_block],
        FunctionId(0),
    );

    let error = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
    )
    .expect_err("unsupported host call should produce diagnostic");
    let (_error_type, message, _location) = error
        .first_infrastructure_error_for_tests()
        .expect("Wasm lowering failure should be wrapped for rendering");
    assert!(message.contains("<synthetic>"));
}

#[test]
fn export_reachable_policy_ignores_unreachable_host_calls() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let start_path = InternedPath::from_single_str("main", &mut string_table);
    let unused_path = InternedPath::from_single_str("unused", &mut string_table);

    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(unit_expression(900, types.unit, RegionId(0))),
    };

    let unsupported_id =
        crate::compiler_frontend::external_packages::ExternalFunctionId::Synthetic(9999);
    let unused_block = HirBlock {
        id: BlockId(10),
        region: RegionId(0),
        locals: vec![],
        statements: vec![statement(
            1,
            HirStatementKind::Call {
                target: CallTarget::ExternalFunction(unsupported_id),
                args: vec![],
                result: None,
            },
            1,
        )],
        terminator: HirTerminator::Return(unit_expression(901, types.unit, RegionId(0))),
    };

    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
        return_aliases: vec![],
    };
    let unused_function = HirFunction {
        id: FunctionId(1),
        entry: BlockId(10),
        params: vec![],
        return_type: types.unit,
        return_aliases: vec![],
    };
    let module = build_module(
        &mut string_table,
        vec![
            (start_function, start_path, HirFunctionOrigin::EntryStart),
            (unused_function, unused_path, HirFunctionOrigin::Normal),
        ],
        vec![start_block, unused_block],
        FunctionId(0),
    );

    let mut export_names = FxHashMap::default();
    export_names.insert(FunctionId(0), "main".to_owned());
    let request = WasmBackendRequest {
        export_policy: WasmExportPolicy {
            exported_functions: vec![FunctionId(0)],
            export_names,
            helper_exports: Default::default(),
        },
        function_emission_policy: WasmFunctionEmissionPolicy::ReachableFromExports,
        ..Default::default()
    };

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &request,
        &string_table,
        &type_environment,
    )
    .expect("unreachable unsupported host call should not be lowered");

    assert_eq!(result.lir_module.imports.len(), 0);
    assert_eq!(
        result.lir_module.functions.len(),
        2,
        "only the exported start function and its wrapper should be lowered"
    );
}

#[test]
fn lower_type_to_abi_maps_all_hir_types_correctly() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let start_path = InternedPath::from_single_str("main", &mut string_table);

    // Build a minimal module so we can construct a WasmLirLoweringContext.
    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(1000, 0, types.int, RegionId(0))),
    };
    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.int,
        return_aliases: vec![],
    };
    let module = build_module(
        &mut string_table,
        vec![(start_function, start_path, HirFunctionOrigin::EntryStart)],
        vec![start_block],
        FunctionId(0),
    );

    // Additional builtin types to test all ABI mappings.
    let builtins = type_environment.builtins();
    let float_id = builtins.float;
    let char_id = builtins.char;
    // Decimal is intentionally inactive in the Alpha surface; it must not lower to a
    // numeric ABI type even though the builtin TypeId still exists.
    let decimal_id = builtins.decimal;
    let range_id = builtins.range;

    let borrow_facts = default_borrow_facts();
    let request = WasmBackendRequest::default();
    let context = crate::backends::wasm::hir_to_lir::context::WasmLirLoweringContext::new(
        &module,
        &borrow_facts,
        &request,
        &string_table,
        &type_environment,
    );

    assert_eq!(lower_type_to_abi(&context, types.int), WasmAbiType::I64);
    assert_eq!(lower_type_to_abi(&context, types.boolean), WasmAbiType::I32);
    assert_eq!(
        lower_type_to_abi(&context, types.string),
        WasmAbiType::Handle
    );
    assert_eq!(lower_type_to_abi(&context, types.unit), WasmAbiType::Void);
    assert_eq!(lower_type_to_abi(&context, float_id), WasmAbiType::F64);
    assert_eq!(lower_type_to_abi(&context, char_id), WasmAbiType::I32);
    assert_eq!(lower_type_to_abi(&context, decimal_id), WasmAbiType::Handle);
    assert_eq!(lower_type_to_abi(&context, range_id), WasmAbiType::Handle);
}

#[test]
fn multi_fragment_template_produces_all_push_operations() {
    // Verifies that a runtime template with literal + handle + literal + handle
    // produces the correct sequence: NewBuffer, PushLiteral, PushHandle, PushLiteral, PushHandle, Finish.
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let runtime_path = InternedPath::from_single_str("__moth_frag_0", &mut string_table);

    // Build: "prefix" + param0 + "middle" + param1 + "suffix"
    let inner_concat_1 = expression(
        1101,
        HirExpressionKind::BinOp {
            left: Box::new(string_expression(1100, "prefix", types.string, RegionId(0))),
            op: HirBinOp::Add,
            right: Box::new(load_local(1102, LocalId(0), types.string, RegionId(0))),
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
    );
    let inner_concat_2 = expression(
        1103,
        HirExpressionKind::BinOp {
            left: Box::new(inner_concat_1),
            op: HirBinOp::Add,
            right: Box::new(string_expression(1104, "middle", types.string, RegionId(0))),
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
    );
    let inner_concat_3 = expression(
        1105,
        HirExpressionKind::BinOp {
            left: Box::new(inner_concat_2),
            op: HirBinOp::Add,
            right: Box::new(load_local(1106, LocalId(1), types.string, RegionId(0))),
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
    );
    let full_concat = expression(
        1107,
        HirExpressionKind::BinOp {
            left: Box::new(inner_concat_3),
            op: HirBinOp::Add,
            right: Box::new(string_expression(1108, "suffix", types.string, RegionId(0))),
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
    );

    let runtime_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![
            local(0, types.string, RegionId(0)),
            local(1, types.string, RegionId(0)),
        ],
        statements: vec![],
        terminator: HirTerminator::Return(full_concat),
    };

    let runtime_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![LocalId(0), LocalId(1)],
        return_type: types.string,
        return_aliases: vec![],
    };

    let module = build_module(
        &mut string_table,
        vec![(runtime_function, runtime_path, HirFunctionOrigin::Normal)],
        vec![runtime_block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
    )
    .expect("multi-fragment template should lower successfully");

    let runtime_lir = result
        .lir_module
        .functions
        .iter()
        .find(|f| f.id == WasmLirFunctionId(0))
        .expect("runtime function should be present");
    let stmts = &runtime_lir.blocks[0].statements;

    // Expected order: NewBuffer, PushLiteral("prefix"), PushHandle(param0),
    // PushLiteral("middle"), PushHandle(param1), PushLiteral("suffix"), Finish
    assert!(matches!(stmts[0], WasmLirStmt::StringNewBuffer { .. }));
    assert!(matches!(stmts[1], WasmLirStmt::StringPushLiteral { .. }));
    assert!(matches!(stmts[2], WasmLirStmt::StringPushHandle { .. }));
    assert!(matches!(stmts[3], WasmLirStmt::StringPushLiteral { .. }));
    assert!(matches!(stmts[4], WasmLirStmt::StringPushHandle { .. }));
    assert!(matches!(stmts[5], WasmLirStmt::StringPushLiteral { .. }));
    assert!(matches!(stmts[6], WasmLirStmt::StringFinish { .. }));

    // Three distinct string literals should produce at least 3 static data entries.
    assert!(result.lir_module.static_data.len() >= 3);
}

#[test]
fn debug_name_uses_source_name_when_available() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let fn_path = InternedPath::from_single_str("my_helper", &mut string_table);

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(1200, 0, types.int, RegionId(0))),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.int,
        return_aliases: vec![],
    };
    let module = build_module(
        &mut string_table,
        vec![(function, fn_path, HirFunctionOrigin::Normal)],
        vec![block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
    )
    .expect("lowering should succeed");

    let lir_fn = result
        .lir_module
        .functions
        .iter()
        .find(|f| f.id == WasmLirFunctionId(0))
        .expect("function should be present");
    assert!(
        lir_fn.debug_name.contains("my_helper"),
        "debug name should contain source name, got: {}",
        lir_fn.debug_name
    );
}

// ---------------------------------------------------------------------------
// Host function unsupported-in-Wasm tests
// ---------------------------------------------------------------------------

/// Verifies that V1 console functions are not silently mapped to a Wasm host import.
/// WHAT: the old callable IO path used to hardcode a Wasm host import.
/// WHY: new console functions have no Wasm lowering and must fail through the normal
/// unsupported-external-function validation path.
#[test]
fn io_console_functions_are_unsupported_in_wasm() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let main_path = InternedPath::from_single_str("main", &mut string_table);

    let io_call = statement(
        1,
        HirStatementKind::Call {
            target: CallTarget::ExternalFunction(
                crate::compiler_frontend::external_packages::ExternalFunctionId::IoLine,
            ),
            args: vec![string_expression(2, "hello", types.string, RegionId(0))],
            result: None,
        },
        1,
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![io_call],
        terminator: HirTerminator::Return(unit_expression(3, types.unit, RegionId(0))),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
        return_aliases: vec![],
    };

    let module = build_module(
        &mut string_table,
        vec![(function, main_path, HirFunctionOrigin::EntryStart)],
        vec![block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
    );

    assert!(
        result.is_err(),
        "Wasm lowering must reject unsupported V1 IO console functions"
    );
    let error_message = format!("{:?}", result.unwrap_err());
    assert!(
        error_message.contains("does not yet support host function"),
        "error should report unsupported host function, got: {error_message}"
    );
    assert!(
        !error_message.contains("host import"),
        "error must not suggest that a host import lowering exists, got: {error_message}"
    );
}
