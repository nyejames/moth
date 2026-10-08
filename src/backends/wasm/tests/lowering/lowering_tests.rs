use crate::backends::wasm::backend::lower_hir_to_wasm_lir;
use crate::backends::wasm::hir_to_lir::context::lower_type_to_abi;
use crate::backends::wasm::lir::function::WasmLirFunctionOrigin;
use crate::backends::wasm::lir::instructions::{
    WasmCalleeRef, WasmLirStmt, WasmLirTerminator, WasmScalarComparisonOp, WasmScalarComparisonType,
};
use crate::backends::wasm::lir::linkage::{WasmExportKind, WasmFunctionLinkage, WasmImportKind};
use crate::backends::wasm::lir::types::{
    WasmAbiType, WasmLirFunctionId, WasmLirLocalId, WasmLocalRole,
};
use crate::backends::wasm::request::{
    WasmBackendRequest, WasmDebugFlags, WasmExportPolicy, WasmFunctionEmissionPolicy,
};
use crate::backends::wasm::tests::lowering::test_support::{
    bool_expression, borrow_facts_with_drop_site, build_module, build_type_environment,
    default_borrow_facts, default_numeric_proofs, expression, int_expression, load_local, local,
    statement, string_expression, unit_expression,
};
use crate::compiler_frontend::analysis::borrow_checker::BorrowDropSiteKind;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expression_store::{
    HirExpressionStore, HirValueRange, HirVariantFieldRange,
};
use crate::compiler_frontend::hir::expressions::{
    HirExpressionKind, HirVariantCarrier, HirVariantField, ValueKind,
};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use moth_lexical::numeric::precision::BinaryFloatPrecision;
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};

use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::numeric::NumericFailureMode;
use crate::compiler_frontend::hir::operators::HirBinOp;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{
    HirAssertionMessageEvaluation, HirJumpArgument, HirTerminator,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::FxHashMap;

fn assertion_failure_module(
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
    type_environment: &mut crate::compiler_frontend::datatypes::environment::TypeEnvironment,
    message_evaluation: HirAssertionMessageEvaluation,
) -> crate::compiler_frontend::hir::module::HirModule {
    let string_type = type_environment.builtins().string;
    let unit_type = type_environment.builtins().none;
    let option_string = type_environment.intern_option(string_type);
    let path = path_fork
        .try_intern_portable_path("assertion_failure", string_table)
        .expect("test path fits");
    let region = RegionId(0);
    let mut expressions = HirExpressionStore::default();
    let (message, locals, statements) = match message_evaluation {
        HirAssertionMessageEvaluation::Default => {
            let message = expression(
                HirExpressionKind::VariantConstruct {
                    carrier: HirVariantCarrier::Option,
                    variant_index: 0,
                    fields: HirVariantFieldRange::empty(),
                },
                option_string,
                region,
                ValueKind::Const,
                &mut expressions,
            );
            (message, vec![], vec![])
        }
        HirAssertionMessageEvaluation::Folded => {
            let folded_message =
                string_expression("folded message", string_type, region, &mut expressions);
            let fields = expressions
                .append_variant_fields(
                    &[HirVariantField {
                        name: None,
                        value: folded_message,
                    }],
                    None,
                )
                .expect("assertion message field should fit");
            let message = expression(
                HirExpressionKind::VariantConstruct {
                    carrier: HirVariantCarrier::Option,
                    variant_index: 1,
                    fields,
                },
                option_string,
                region,
                ValueKind::RValue,
                &mut expressions,
            );
            (message, vec![], vec![])
        }
        HirAssertionMessageEvaluation::Runtime => {
            let runtime_message = load_local(&mut expressions, LocalId(0), string_type, region);
            let assigned_message =
                string_expression("runtime message", string_type, region, &mut expressions);
            let fields = expressions
                .append_variant_fields(
                    &[HirVariantField {
                        name: None,
                        value: runtime_message,
                    }],
                    None,
                )
                .expect("assertion message field should fit");
            let message = expression(
                HirExpressionKind::VariantConstruct {
                    carrier: HirVariantCarrier::Option,
                    variant_index: 1,
                    fields,
                },
                option_string,
                region,
                ValueKind::RValue,
                &mut expressions,
            );
            let statements = vec![statement(
                3,
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(LocalId(0)),
                    value: assigned_message,
                },
                1,
            )];
            (message, vec![local(0, string_type, region)], statements)
        }
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: unit_type,
    };
    let block = HirBlock {
        id: BlockId(0),
        region,
        locals,
        statements,
        terminator: HirTerminator::AssertFailure {
            message,
            message_evaluation,
        },
    };

    build_module(
        path_fork,
        string_table,
        expressions,
        vec![(function, path, HirFunctionOrigin::EntryStart)],
        vec![block],
        FunctionId(0),
    )
}

#[test]
fn wasm_assertion_lowering_calls_host_before_trap_and_rejects_runtime_messages() {
    for message_evaluation in [
        HirAssertionMessageEvaluation::Default,
        HirAssertionMessageEvaluation::Folded,
    ] {
        let mut string_table = StringTable::new();
        let mut path_fork = PathInternerFork::empty();
        let (mut type_environment, _types) = build_type_environment();
        let module = assertion_failure_module(
            &mut path_fork,
            &mut string_table,
            &mut type_environment,
            message_evaluation,
        );
        let result = lower_hir_to_wasm_lir(
            &module,
            &default_borrow_facts(),
            &default_numeric_proofs(),
            &WasmBackendRequest::default(),
            &string_table,
            &type_environment,
            &path_fork.snapshot_table(),
        )
        .expect("static assertion messages should lower to a host call and trap");
        let function = result
            .lir_module
            .functions
            .iter()
            .find(|function| function.id == WasmLirFunctionId(0))
            .expect("assertion function should be present");
        assert!(matches!(
            function.blocks[0].terminator,
            WasmLirTerminator::Trap
        ));
        let [import] = result.lir_module.imports.as_slice() else {
            panic!("assertions should demand exactly one host import");
        };
        assert_eq!(import.module_name, "host");
        assert_eq!(import.item_name, "assertion_failed");
        let WasmImportKind::Function(signature) = &import.kind;
        assert_eq!(signature.params, [WasmAbiType::Handle]);
        assert!(signature.results.is_empty());
        assert!(matches!(
            function.blocks[0].statements.last(),
            Some(WasmLirStmt::Call {
                dst: None,
                callee: WasmCalleeRef::Import(id),
                args,
            }) if *id == import.id && args.len() == 1
        ));
    }

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (mut type_environment, _types) = build_type_environment();
    let module = assertion_failure_module(
        &mut path_fork,
        &mut string_table,
        &mut type_environment,
        HirAssertionMessageEvaluation::Runtime,
    );
    let error = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect_err("runtime assertion messages must be rejected before Wasm lowering");
    assert!(
        format!("{error:?}").contains("runtime assertion message after target validation"),
        "the lowerer must report the backend invariant violation instead of discarding the message"
    );
}

#[test]
fn wasm_write_definition_and_update_have_the_same_physical_lowering() {
    let lower_write = |target| {
        let is_update = matches!(&target, HirWriteTarget::AssignPlace(_));
        let mut string_table = StringTable::new();
        let mut path_fork = PathInternerFork::empty();
        let mut expressions = HirExpressionStore::default();
        let (type_environment, types) = build_type_environment();
        let function_path = path_fork
            .try_intern_portable_path("write_equivalence", &mut string_table)
            .expect("test path fits");
        let write = statement(
            1,
            HirStatementKind::Write {
                target,
                value: int_expression(7, types.int, RegionId(0), &mut expressions),
            },
            1,
        );
        let block = HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![local(0, types.int, RegionId(0))],
            statements: vec![write],
            terminator: HirTerminator::Return(unit_expression(
                types.unit,
                RegionId(0),
                &mut expressions,
            )),
        };
        let function = HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: if is_update { vec![LocalId(0)] } else { vec![] },
            return_type: types.unit,
        };
        let module = build_module(
            &mut path_fork,
            &mut string_table,
            expressions,
            vec![(function, function_path, HirFunctionOrigin::Normal)],
            vec![block],
            FunctionId(0),
        );
        let lowered = lower_hir_to_wasm_lir(
            &module,
            &default_borrow_facts(),
            &default_numeric_proofs(),
            &WasmBackendRequest::default(),
            &string_table,
            &type_environment,
            &path_fork.snapshot_table(),
        )
        .expect("both valid local-write forms should lower");
        lowered
            .lir_module
            .functions
            .iter()
            .find(|function| function.id == WasmLirFunctionId(0))
            .expect("write function should be present")
            .blocks[0]
            .statements
            .clone()
    };

    let definition = lower_write(HirWriteTarget::DefineLocal(LocalId(0)));
    let update = lower_write(HirWriteTarget::AssignPlace(HirPlace::local(LocalId(0))));
    assert_eq!(definition, update);
}

#[test]
fn wasm_parallel_jump_cycle_uses_explicit_destinations() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let function_path = path_fork
        .try_intern_portable_path("parallel_jump_cycle", &mut string_table)
        .expect("test path fits");
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };
    let source_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![
            local(0, types.int, RegionId(0)),
            local(1, types.int, RegionId(0)),
        ],
        statements: vec![
            statement(
                1,
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(LocalId(0)),
                    value: int_expression(10, types.int, RegionId(0), &mut expressions),
                },
                1,
            ),
            statement(
                2,
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(LocalId(1)),
                    value: int_expression(20, types.int, RegionId(0), &mut expressions),
                },
                2,
            ),
        ],
        terminator: HirTerminator::Jump {
            target: BlockId(0),
            args: vec![
                HirJumpArgument {
                    source: LocalId(0),
                    destination: LocalId(1),
                },
                HirJumpArgument {
                    source: LocalId(1),
                    destination: LocalId(0),
                },
            ],
        },
    };
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![(function, function_path, HirFunctionOrigin::Normal)],
        vec![source_block],
        FunctionId(0),
    );

    let lowered = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("parallel explicit jump destinations should lower");
    let function = lowered
        .lir_module
        .functions
        .iter()
        .find(|function| function.id == WasmLirFunctionId(0))
        .expect("jump function should be present");
    let local_zero = function
        .locals
        .iter()
        .find(|local| local.name.as_deref() == Some("local_0"))
        .expect("local zero should be lowered")
        .id;
    let local_one = function
        .locals
        .iter()
        .find(|local| local.name.as_deref() == Some("local_1"))
        .expect("local one should be lowered")
        .id;
    let [
        WasmLirStmt::Copy {
            dst: snapshot,
            src: snapshot_source,
        },
        WasmLirStmt::Copy {
            dst: first_destination,
            src: first_source,
        },
        WasmLirStmt::Copy {
            dst: second_destination,
            src: second_source,
        },
    ] = function.blocks[0].statements[function.blocks[0].statements.len() - 3..]
    else {
        panic!("a parallel two-local cycle needs one snapshot and two copies");
    };

    assert_eq!(snapshot_source, local_one);
    assert_eq!(first_destination, local_one);
    assert_eq!(first_source, local_zero);
    assert_eq!(second_destination, local_zero);
    assert_eq!(second_source, snapshot);
}

#[test]
fn lowers_calls_and_cfg_with_resolvable_branch_targets() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();

    let callee_path = path_fork
        .try_intern_portable_path("callee", &mut string_table)
        .expect("test path fits");
    let main_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");

    let callee_block = HirBlock {
        id: BlockId(10),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(
            7,
            types.int,
            RegionId(0),
            &mut expressions,
        )),
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
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(LocalId(0)),
                    value: bool_expression(true, types.boolean, RegionId(0), &mut expressions),
                },
                1,
            ),
            statement(
                2,
                HirStatementKind::Call {
                    target: CallTarget::Local(FunctionId(0)),
                    args: HirValueRange::empty(),
                    result: Some(HirLocalDestination::Define(LocalId(1))),
                },
                2,
            ),
        ],
        terminator: HirTerminator::If {
            condition: load_local(&mut expressions, LocalId(0), types.boolean, RegionId(0)),
            then_block: BlockId(40),
            else_block: BlockId(50),
        },
    };

    let then_block = HirBlock {
        id: BlockId(40),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(load_local(
            &mut expressions,
            LocalId(1),
            types.int,
            RegionId(0),
        )),
    };

    let else_block = HirBlock {
        id: BlockId(50),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(
            0,
            types.int,
            RegionId(0),
            &mut expressions,
        )),
    };

    let callee = HirFunction {
        id: FunctionId(0),
        entry: BlockId(10),
        params: vec![],
        return_type: types.int,
    };
    let main = HirFunction {
        id: FunctionId(1),
        entry: BlockId(30),
        params: vec![],
        return_type: types.int,
    };

    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
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
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
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
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let runtime_path = path_fork
        .try_intern_portable_path("__moth_frag_0", &mut string_table)
        .expect("test path fits");

    let prefix = string_expression("a", types.string, RegionId(0), &mut expressions);
    let handle = load_local(&mut expressions, LocalId(0), types.string, RegionId(0));
    let partial_concat = expression(
        HirExpressionKind::BinOp {
            left: prefix,
            op: HirBinOp::StringAppend,
            right: handle,
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );
    let suffix = string_expression("b", types.string, RegionId(0), &mut expressions);
    let concat = expression(
        HirExpressionKind::BinOp {
            left: partial_concat,
            op: HirBinOp::StringAppend,
            right: suffix,
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
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
    };

    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![(runtime_function, runtime_path, HirFunctionOrigin::Normal)],
        vec![runtime_block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
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
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let runtime_path = path_fork
        .try_intern_portable_path("__moth_frag_cfg", &mut string_table)
        .expect("test path fits");
    let initial_value = int_expression(0, types.int, RegionId(0), &mut expressions);
    let loop_value = load_local(&mut expressions, LocalId(0), types.int, RegionId(0));
    let limit_value = int_expression(2, types.int, RegionId(0), &mut expressions);
    let condition = expression(
        HirExpressionKind::BinOp {
            left: loop_value,
            op: HirBinOp::Lt,
            right: limit_value,
        },
        types.boolean,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );
    let increment_value = load_local(&mut expressions, LocalId(0), types.int, RegionId(0));
    let return_value = string_expression(
        "runtime loop done",
        types.string,
        RegionId(0),
        &mut expressions,
    );

    let entry_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![statement(
            300,
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(LocalId(0)),
                value: initial_value,
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
            condition,
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
            HirStatementKind::Write {
                target: HirWriteTarget::AssignPlace(HirPlace::local(LocalId(0))),
                value: increment_value,
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
        terminator: HirTerminator::Return(return_value),
    };

    let runtime_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.string,
    };

    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![(runtime_function, runtime_path, HirFunctionOrigin::Normal)],
        vec![entry_block, header_block, body_block, exit_block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
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
fn lowers_internal_string_append_as_buffer_concat() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let function_path = path_fork
        .try_intern_portable_path("render_title", &mut string_table)
        .expect("test path fits");

    let prefix = string_expression("Title: ", types.string, RegionId(0), &mut expressions);
    let handle = load_local(&mut expressions, LocalId(0), types.string, RegionId(0));
    let concat = expression(
        HirExpressionKind::BinOp {
            left: prefix,
            op: HirBinOp::StringAppend,
            right: handle,
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
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
    };
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![(function, function_path, HirFunctionOrigin::Normal)],
        vec![block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("internal StringAppend should lower to buffer operations");

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
fn lowers_internal_string_append_with_i64_chunk_via_string_from_i64() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let function_path = path_fork
        .try_intern_portable_path("render_runtime_int", &mut string_table)
        .expect("test path fits");

    let prefix = string_expression("", types.string, RegionId(0), &mut expressions);
    let handle = load_local(&mut expressions, LocalId(0), types.int, RegionId(0));
    let concat = expression(
        HirExpressionKind::BinOp {
            left: prefix,
            op: HirBinOp::StringAppend,
            right: handle,
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
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
    };
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![(function, function_path, HirFunctionOrigin::Normal)],
        vec![block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("internal StringAppend should bridge i64 chunks");
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
fn lowers_string_equality_by_content_comparison_operations() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let function_path = path_fork
        .try_intern_portable_path("compare_strings", &mut string_table)
        .expect("test path fits");

    let equal_left = load_local(&mut expressions, LocalId(0), types.string, RegionId(0));
    let equal_right = load_local(&mut expressions, LocalId(1), types.string, RegionId(0));
    let equal = expression(
        HirExpressionKind::BinOp {
            left: equal_left,
            op: HirBinOp::Eq,
            right: equal_right,
        },
        types.boolean,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );
    let not_equal_left = load_local(&mut expressions, LocalId(0), types.string, RegionId(0));
    let not_equal_right = load_local(&mut expressions, LocalId(1), types.string, RegionId(0));
    let not_equal = expression(
        HirExpressionKind::BinOp {
            left: not_equal_left,
            op: HirBinOp::Ne,
            right: not_equal_right,
        },
        types.boolean,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );
    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![
            local(0, types.string, RegionId(0)),
            local(1, types.string, RegionId(0)),
            local(2, types.boolean, RegionId(0)),
            local(3, types.boolean, RegionId(0)),
        ],
        statements: vec![
            statement(
                1326,
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(LocalId(2)),
                    value: equal,
                },
                1326,
            ),
            statement(
                1327,
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(LocalId(3)),
                    value: not_equal,
                },
                1327,
            ),
        ],
        terminator: HirTerminator::Return(load_local(
            &mut expressions,
            LocalId(3),
            types.boolean,
            RegionId(0),
        )),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![LocalId(0), LocalId(1)],
        return_type: types.boolean,
    };
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![(function, function_path, HirFunctionOrigin::Normal)],
        vec![block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("String equality should lower through content comparison operations");
    let lowered = result
        .lir_module
        .functions
        .iter()
        .find(|function| function.id == WasmLirFunctionId(0))
        .expect("lowered function should be present");
    let statements = &lowered.blocks[0].statements;

    assert!(
        statements
            .iter()
            .any(|statement| matches!(statement, WasmLirStmt::StringEq { .. })),
        "String equality should not compare handle addresses"
    );
    assert!(
        statements
            .iter()
            .any(|statement| matches!(statement, WasmLirStmt::StringNe { .. })),
        "String inequality should negate content equality"
    );
    assert!(
        !statements.iter().any(|statement| matches!(
            statement,
            WasmLirStmt::IntEq { .. } | WasmLirStmt::IntNe { .. }
        )),
        "String equality must not use integer handle comparison"
    );
}

#[test]
fn lowers_ordered_comparison_and_control_flow() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let function_path = path_fork
        .try_intern_portable_path("loop_like_fn", &mut string_table)
        .expect("test path fits");

    let comparison_left = load_local(&mut expressions, LocalId(0), types.int, RegionId(0));
    let comparison_right = int_expression(5, types.int, RegionId(0), &mut expressions);
    let condition = expression(
        HirExpressionKind::BinOp {
            left: comparison_left,
            op: HirBinOp::Le,
            right: comparison_right,
        },
        types.boolean,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );

    let entry_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![statement(
            1,
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(LocalId(0)),
                value: int_expression(0, types.int, RegionId(0), &mut expressions),
            },
            1,
        )],
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
        statements: vec![],
        terminator: HirTerminator::Return(load_local(
            &mut expressions,
            LocalId(0),
            types.int,
            RegionId(0),
        )),
    };
    let else_block = HirBlock {
        id: BlockId(2),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(
            0,
            types.int,
            RegionId(0),
            &mut expressions,
        )),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.int,
    };
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![(function, function_path, HirFunctionOrigin::Normal)],
        vec![entry_block, then_block, else_block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("ordered comparison and control flow should lower");

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
            .any(|statement| matches!(
                statement,
                WasmLirStmt::ScalarCompare {
                    op: WasmScalarComparisonOp::Le,
                    lhs_type: WasmScalarComparisonType::SignedInteger(32),
                    rhs_type: WasmScalarComparisonType::SignedInteger(32),
                    ..
                }
            )),
        "entry block should include profile-aware signed scalar comparison lowering"
    );
    assert!(
        matches!(
            lowered.blocks[0].terminator,
            WasmLirTerminator::Branch { .. }
        ),
        "entry block should preserve the conditional control flow"
    );
}

#[test]
fn deduplicates_static_utf8_segments() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let start_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");

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
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(LocalId(0)),
                    value: string_expression("same", types.string, RegionId(0), &mut expressions),
                },
                1,
            ),
            statement(
                2,
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(LocalId(1)),
                    value: string_expression("same", types.string, RegionId(0), &mut expressions),
                },
                2,
            ),
        ],
        terminator: HirTerminator::Return(unit_expression(
            types.unit,
            RegionId(0),
            &mut expressions,
        )),
    };

    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![(start_function, start_path, HirFunctionOrigin::EntryStart)],
        vec![start_block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("Wasm lowering should succeed");
    assert_eq!(result.lir_module.static_data.len(), 1);
}

#[test]
fn maps_advisory_drop_sites_to_drop_if_owned_statements() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let start_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");

    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.string, RegionId(0))],
        statements: vec![],
        terminator: HirTerminator::Return(unit_expression(
            types.unit,
            RegionId(0),
            &mut expressions,
        )),
    };
    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![(start_function, start_path, HirFunctionOrigin::EntryStart)],
        vec![start_block],
        FunctionId(0),
    );

    let borrow_facts =
        borrow_facts_with_drop_site(BlockId(0), BorrowDropSiteKind::Return, vec![LocalId(0)]);

    let result = lower_hir_to_wasm_lir(
        &module,
        &borrow_facts,
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
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
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let start_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");

    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(
            123,
            types.int,
            RegionId(0),
            &mut expressions,
        )),
    };
    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.int,
    };
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
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
        structural_string_urls: None,
        function_emission_policy: Default::default(),
        numeric_profile: NumericProfile::STANDARD,
    };

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &request,
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
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
fn exported_f16_parameters_are_rounded_before_the_internal_call() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let f16_type = builtin_type_ids::fixed_scalar(FixedScalar::F16);
    let start_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");
    let exported_path = path_fork
        .try_intern_portable_path("process_half", &mut string_table)
        .expect("test path fits");

    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.int,
    };
    let exported_function = HirFunction {
        id: FunctionId(1),
        entry: BlockId(1),
        params: vec![LocalId(10)],
        return_type: f16_type,
    };
    let start_value = int_expression(0, types.int, RegionId(0), &mut expressions);
    let exported_value = load_local(&mut expressions, LocalId(10), f16_type, RegionId(0));
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![
            (start_function, start_path, HirFunctionOrigin::EntryStart),
            (exported_function, exported_path, HirFunctionOrigin::Normal),
        ],
        vec![
            HirBlock {
                id: BlockId(0),
                region: RegionId(0),
                locals: vec![],
                statements: vec![],
                terminator: HirTerminator::Return(start_value),
            },
            HirBlock {
                id: BlockId(1),
                region: RegionId(0),
                locals: vec![local(10, f16_type, RegionId(0))],
                statements: vec![],
                terminator: HirTerminator::Return(exported_value),
            },
        ],
        FunctionId(0),
    );

    let mut export_names = FxHashMap::default();
    export_names.insert(FunctionId(1), "process_half".to_owned());
    let request = WasmBackendRequest {
        export_policy: WasmExportPolicy {
            exported_functions: vec![FunctionId(1)],
            export_names,
            helper_exports: Default::default(),
        },
        ..Default::default()
    };
    let lowered = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &request,
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("F16 export wrapper should lower");

    let WasmExportKind::Function(wrapper_id) = lowered.lir_module.exports[0].kind;
    let wrapper = lowered
        .lir_module
        .functions
        .iter()
        .find(|function| function.id == wrapper_id)
        .expect("export wrapper should be present");
    assert_eq!(wrapper.signature.params, vec![WasmAbiType::F32]);
    assert_eq!(wrapper.signature.results, vec![WasmAbiType::F32]);

    let statements = &wrapper.blocks[0].statements;
    let round_index = statements
        .iter()
        .position(|statement| matches!(statement, WasmLirStmt::RoundF16 { .. }))
        .expect("the wrapper should round its external F32 parameter");
    let call_index = statements
        .iter()
        .position(|statement| matches!(statement, WasmLirStmt::Call { .. }))
        .expect("the wrapper should call its internal function");
    assert!(round_index < call_index, "rounding must precede the call");

    let WasmLirStmt::RoundF16 { dst, source } = &statements[round_index] else {
        unreachable!("the round index selects a RoundF16")
    };
    assert_eq!(*source, WasmLirLocalId(0));
    let WasmLirStmt::Call {
        dst: Some(call_result),
        args,
        ..
    } = &statements[call_index]
    else {
        unreachable!("the call index selects a value-returning Call")
    };
    assert_eq!(args, &vec![*dst]);
    assert!(
        matches!(
            &wrapper.blocks[0].terminator,
            WasmLirTerminator::Return { value: Some(value) } if *value == *call_result
        ),
        "the wrapper should return the callee's canonical F32 carrier unchanged"
    );
}

#[test]
fn rejects_invalid_export_request_with_structured_diagnostic() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let start_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");

    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(
            1,
            types.int,
            RegionId(0),
            &mut expressions,
        )),
    };
    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.int,
    };
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
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
        &default_numeric_proofs(),
        &invalid_request,
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect_err("invalid request should produce a lowering diagnostic");
    let error = error
        .infrastructure_error()
        .expect("Wasm lowering failure should be wrapped for rendering");
    assert!(
        error
            .msg
            .contains("missing stable export name for FunctionId(0)")
    );
}

fn validate_float_test_module(
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
    type_environment: &crate::compiler_frontend::datatypes::environment::TypeEnvironment,
) -> crate::compiler_frontend::hir::module::HirModule {
    let mut expressions = HirExpressionStore::default();
    let int_type = type_environment.builtins().int;
    let float_type = type_environment.builtins().float;
    let start_path = path_fork
        .try_intern_portable_path("main", string_table)
        .expect("test path fits");
    let validator_path = path_fork
        .try_intern_portable_path("validate_float", string_table)
        .expect("test path fits");
    let start_value = int_expression(0, int_type, RegionId(0), &mut expressions);
    let source = load_local(&mut expressions, LocalId(10), float_type, RegionId(0));
    let result = load_local(&mut expressions, LocalId(20), float_type, RegionId(0));

    build_module(
        path_fork,
        string_table,
        expressions,
        vec![
            (
                HirFunction {
                    id: FunctionId(0),
                    entry: BlockId(0),
                    params: vec![],
                    return_type: int_type,
                },
                start_path,
                HirFunctionOrigin::EntryStart,
            ),
            (
                HirFunction {
                    id: FunctionId(1),
                    entry: BlockId(1),
                    params: vec![LocalId(10)],
                    return_type: float_type,
                },
                validator_path,
                HirFunctionOrigin::Normal,
            ),
        ],
        vec![
            HirBlock {
                id: BlockId(0),
                region: RegionId(0),
                locals: vec![],
                statements: vec![],
                terminator: HirTerminator::Return(start_value),
            },
            HirBlock {
                id: BlockId(1),
                region: RegionId(0),
                locals: vec![
                    local(10, float_type, RegionId(0)),
                    local(20, float_type, RegionId(0)),
                ],
                statements: vec![statement(
                    102,
                    HirStatementKind::ValidateFloat {
                        source,
                        failure_mode: NumericFailureMode::Trap,
                        result: HirLocalDestination::Define(LocalId(20)),
                    },
                    2,
                )],
                terminator: HirTerminator::Return(result),
            },
        ],
        FunctionId(0),
    )
}

#[test]
fn lowers_validate_float_with_profile_precision_and_local_value_path() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, _) = build_type_environment();
    let module = validate_float_test_module(&mut path_fork, &mut string_table, &type_environment);

    for (float_precision, expected_carrier, expected_precision) in [
        (
            FloatPrecision::Bits32,
            WasmAbiType::F32,
            BinaryFloatPrecision::Binary32,
        ),
        (
            FloatPrecision::Bits64,
            WasmAbiType::F64,
            BinaryFloatPrecision::Binary64,
        ),
    ] {
        let profile = NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision,
        };
        let request = WasmBackendRequest {
            numeric_profile: profile,
            ..Default::default()
        };
        let lowered = lower_hir_to_wasm_lir(
            &module,
            &default_borrow_facts(),
            &default_numeric_proofs(),
            &request,
            &string_table,
            &type_environment,
            &path_fork.snapshot_table(),
        )
        .unwrap_or_else(|error| panic!("{profile} should lower: {error:?}"));
        let function = lowered
            .lir_module
            .functions
            .iter()
            .find(|function| function.id == WasmLirFunctionId(1))
            .expect("Float validator should be lowered");
        let source_local = function
            .locals
            .iter()
            .find(|local| local.role == WasmLocalRole::Param)
            .expect("source Float parameter should have a LIR local");
        let result_local = function
            .locals
            .iter()
            .find(|local| local.role == WasmLocalRole::UserLocal)
            .expect("result Float local should have a LIR local");

        assert_eq!(
            function.signature.params,
            vec![expected_carrier],
            "{profile}"
        );
        assert_eq!(
            function.signature.results,
            vec![expected_carrier],
            "{profile}"
        );
        assert_eq!(source_local.ty, expected_carrier, "{profile}");
        assert_eq!(result_local.ty, expected_carrier, "{profile}");
        assert_eq!(
            function
                .blocks
                .iter()
                .flat_map(|block| &block.statements)
                .filter(|statement| matches!(statement, WasmLirStmt::ValidateFloat { .. }))
                .count(),
            1,
            "{profile} should lower exactly one validation"
        );

        let validation_block = function
            .blocks
            .iter()
            .find(|block| {
                block
                    .statements
                    .iter()
                    .any(|statement| matches!(statement, WasmLirStmt::ValidateFloat { .. }))
            })
            .expect("validation block should be present");
        let validation = validation_block
            .statements
            .iter()
            .find(|statement| matches!(statement, WasmLirStmt::ValidateFloat { .. }))
            .expect("ValidateFloat statement should be present");
        let WasmLirStmt::ValidateFloat {
            dst,
            source,
            precision,
        } = validation
        else {
            unreachable!("the selected statement is ValidateFloat")
        };
        assert_eq!(*source, source_local.id, "{profile}");
        assert_eq!(*dst, result_local.id, "{profile}");
        assert_eq!(*precision, expected_precision, "{profile}");
        assert!(
            matches!(
                &validation_block.terminator,
                WasmLirTerminator::Return { value: Some(value) } if *value == *dst
            ),
            "{profile} should return the validated destination unchanged"
        );
    }
}

#[test]
fn lowers_trap_format_float_with_profile_precision_and_single_source_evaluation() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let float_type = type_environment.builtins().float;
    let start_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");
    let formatter_path = path_fork
        .try_intern_portable_path("format_float", &mut string_table)
        .expect("test path fits");
    let start_value = int_expression(0, types.int, RegionId(0), &mut expressions);
    let float_source = expression(
        HirExpressionKind::Float(1.5),
        float_type,
        RegionId(0),
        ValueKind::Const,
        &mut expressions,
    );
    let formatted_value = load_local(&mut expressions, LocalId(20), types.string, RegionId(0));
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![
            (
                HirFunction {
                    id: FunctionId(0),
                    entry: BlockId(0),
                    params: vec![],
                    return_type: types.int,
                },
                start_path,
                HirFunctionOrigin::EntryStart,
            ),
            (
                HirFunction {
                    id: FunctionId(1),
                    entry: BlockId(1),
                    params: vec![],
                    return_type: types.string,
                },
                formatter_path,
                HirFunctionOrigin::Normal,
            ),
        ],
        vec![
            HirBlock {
                id: BlockId(0),
                region: RegionId(0),
                locals: vec![],
                statements: vec![],
                terminator: HirTerminator::Return(start_value),
            },
            HirBlock {
                id: BlockId(1),
                region: RegionId(0),
                locals: vec![local(20, types.string, RegionId(0))],
                statements: vec![statement(
                    202,
                    HirStatementKind::FormatFloat {
                        source: float_source,
                        failure_mode: NumericFailureMode::Trap,
                        result: HirLocalDestination::Define(LocalId(20)),
                    },
                    2,
                )],
                terminator: HirTerminator::Return(formatted_value),
            },
        ],
        FunctionId(0),
    );

    for (float_precision, expected_carrier, expected_precision) in [
        (
            FloatPrecision::Bits32,
            WasmAbiType::F32,
            BinaryFloatPrecision::Binary32,
        ),
        (
            FloatPrecision::Bits64,
            WasmAbiType::F64,
            BinaryFloatPrecision::Binary64,
        ),
    ] {
        let request = WasmBackendRequest {
            numeric_profile: NumericProfile {
                int_width: IntWidth::Bits64,
                float_precision,
            },
            ..Default::default()
        };
        let lowered = lower_hir_to_wasm_lir(
            &module,
            &default_borrow_facts(),
            &default_numeric_proofs(),
            &request,
            &string_table,
            &type_environment,
            &path_fork.snapshot_table(),
        )
        .unwrap_or_else(|error| panic!("{float_precision:?} FormatFloat should lower: {error:?}"));
        let function = lowered
            .lir_module
            .functions
            .iter()
            .find(|function| function.id == WasmLirFunctionId(1))
            .expect("formatter function should be lowered");
        assert_eq!(function.signature.results, vec![WasmAbiType::Handle]);

        let statements = &function.blocks[0].statements;
        let float_constants = statements
            .iter()
            .filter_map(|statement| match statement {
                WasmLirStmt::ConstF32 { dst, .. } | WasmLirStmt::ConstF64 { dst, .. } => Some(*dst),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            float_constants.len(),
            1,
            "{float_precision:?} source expression should be lowered exactly once"
        );
        assert_eq!(
            statements
                .iter()
                .filter(|statement| matches!(statement, WasmLirStmt::StringFromFloat { .. }))
                .count(),
            1,
            "{float_precision:?} should emit one StringFromFloat"
        );
        let formatted = statements
            .iter()
            .find_map(|statement| match statement {
                WasmLirStmt::StringFromFloat {
                    dst,
                    value,
                    precision,
                } => Some((*dst, *value, *precision)),
                _ => None,
            })
            .expect("FormatFloat should lower through StringFromFloat");
        assert_eq!(formatted.1, float_constants[0]);
        assert_eq!(formatted.2, expected_precision);
        let source_local = function
            .locals
            .iter()
            .find(|local| local.id == formatted.1)
            .expect("formatted source local should exist");
        assert_eq!(source_local.ty, expected_carrier);
        let destination = function
            .locals
            .iter()
            .find(|local| local.role == WasmLocalRole::UserLocal)
            .expect("formatted String result local should exist");
        assert_eq!(destination.ty, WasmAbiType::Handle);
        assert_eq!(formatted.0, destination.id);
    }
}

#[test]
fn lowers_fixed_float_to_string_with_source_precision_and_single_evaluation() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let start_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");
    let formatter_path = path_fork
        .try_intern_portable_path("format_fixed_float", &mut string_table)
        .expect("test path fits");

    for (scalar, expected_precision, expected_carrier) in [
        (
            FixedScalar::F16,
            BinaryFloatPrecision::Binary16,
            WasmAbiType::F32,
        ),
        (
            FixedScalar::F32,
            BinaryFloatPrecision::Binary32,
            WasmAbiType::F32,
        ),
        (
            FixedScalar::F64,
            BinaryFloatPrecision::Binary64,
            WasmAbiType::F64,
        ),
    ] {
        let mut expressions = HirExpressionStore::default();
        let source = expression(
            HirExpressionKind::FixedScalar(
                FixedScalarValue::binary_float(scalar, 1.5)
                    .expect("1.5 is a finite fixed-width float"),
            ),
            builtin_type_ids::fixed_scalar(scalar),
            RegionId(0),
            ValueKind::Const,
            &mut expressions,
        );
        let cast = expression(
            HirExpressionKind::Cast {
                source,
                policy: BuiltinCastPolicyId::NumericToString(NumericScalar::Fixed(scalar)),
            },
            types.string,
            RegionId(0),
            ValueKind::RValue,
            &mut expressions,
        );
        let start_value = int_expression(0, types.int, RegionId(0), &mut expressions);
        let module = build_module(
            &mut path_fork,
            &mut string_table,
            expressions,
            vec![
                (
                    HirFunction {
                        id: FunctionId(0),
                        entry: BlockId(0),
                        params: vec![],
                        return_type: types.int,
                    },
                    start_path,
                    HirFunctionOrigin::EntryStart,
                ),
                (
                    HirFunction {
                        id: FunctionId(1),
                        entry: BlockId(1),
                        params: vec![],
                        return_type: types.string,
                    },
                    formatter_path,
                    HirFunctionOrigin::Normal,
                ),
            ],
            vec![
                HirBlock {
                    id: BlockId(0),
                    region: RegionId(0),
                    locals: vec![],
                    statements: vec![],
                    terminator: HirTerminator::Return(start_value),
                },
                HirBlock {
                    id: BlockId(1),
                    region: RegionId(0),
                    locals: vec![],
                    statements: vec![],
                    terminator: HirTerminator::Return(cast),
                },
            ],
            FunctionId(0),
        );
        let lowered = lower_hir_to_wasm_lir(
            &module,
            &default_borrow_facts(),
            &default_numeric_proofs(),
            &WasmBackendRequest::default(),
            &string_table,
            &type_environment,
            &path_fork.snapshot_table(),
        )
        .unwrap_or_else(|error| panic!("{scalar:?} NumericToString should lower: {error:?}"));
        let function = lowered
            .lir_module
            .functions
            .iter()
            .find(|function| function.id == WasmLirFunctionId(1))
            .expect("fixed-float formatter should be lowered");
        assert_eq!(function.signature.results, vec![WasmAbiType::Handle]);
        let statements = &function.blocks[0].statements;
        let float_constants = statements
            .iter()
            .filter_map(|statement| match statement {
                WasmLirStmt::ConstF32 { dst, .. } | WasmLirStmt::ConstF64 { dst, .. } => Some(*dst),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            float_constants.len(),
            1,
            "{scalar:?} source expression should be lowered exactly once"
        );
        assert_eq!(
            statements
                .iter()
                .filter(|statement| matches!(statement, WasmLirStmt::StringFromFloat { .. }))
                .count(),
            1,
            "{scalar:?} should emit one StringFromFloat"
        );
        let formatted = statements
            .iter()
            .find_map(|statement| match statement {
                WasmLirStmt::StringFromFloat {
                    dst,
                    value,
                    precision,
                } => Some((*dst, *value, *precision)),
                _ => None,
            })
            .expect("NumericToString should lower through StringFromFloat");
        assert_eq!(formatted.1, float_constants[0]);
        assert_eq!(formatted.2, expected_precision);
        assert_eq!(
            function
                .locals
                .iter()
                .find(|local| local.id == formatted.1)
                .expect("formatted input local should exist")
                .ty,
            expected_carrier
        );
        assert_eq!(
            function
                .locals
                .iter()
                .find(|local| local.id == formatted.0)
                .expect("String result local should exist")
                .ty,
            WasmAbiType::Handle
        );
    }
}

#[test]
fn lowers_every_numeric_profile_with_selected_int_carrier() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let start_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");

    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(
            123,
            types.int,
            RegionId(0),
            &mut expressions,
        )),
    };
    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.int,
    };
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![(start_function, start_path, HirFunctionOrigin::EntryStart)],
        vec![start_block],
        FunctionId(0),
    );

    for profile in [
        NumericProfile::STANDARD,
        NumericProfile {
            int_width: IntWidth::Bits32,
            float_precision: FloatPrecision::Bits32,
        },
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits64,
        },
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits32,
        },
    ] {
        let request = WasmBackendRequest {
            numeric_profile: profile,
            ..Default::default()
        };
        let lowered = lower_hir_to_wasm_lir(
            &module,
            &default_borrow_facts(),
            &default_numeric_proofs(),
            &request,
            &string_table,
            &type_environment,
            &path_fork.snapshot_table(),
        )
        .unwrap_or_else(|error| panic!("{profile} should lower: {error:?}"));
        let start = lowered
            .lir_module
            .functions
            .iter()
            .find(|function| function.origin == WasmLirFunctionOrigin::EntryStart)
            .expect("the start function should be lowered");
        let expected = match profile.int_width {
            IntWidth::Bits32 => WasmAbiType::I32,
            IntWidth::Bits64 => WasmAbiType::I64,
        };
        assert_eq!(start.signature.results, vec![expected], "{profile}");
        assert!(
            start.blocks[0].statements.iter().any(|statement| matches!(
                (expected, statement),
                (WasmAbiType::I32, WasmLirStmt::ConstI32 { value: 123, .. })
                    | (WasmAbiType::I64, WasmLirStmt::ConstI64 { value: 123, .. })
            )),
            "{profile} Int literal must use the selected carrier"
        );
    }
}

#[test]
fn rejects_unsupported_host_call_with_diagnostic() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let start_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");

    let unknown_id =
        crate::compiler_frontend::external_packages::ExternalFunctionId::Synthetic(9999);
    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![statement(
            1,
            HirStatementKind::Call {
                target: CallTarget::External(unknown_id),
                args: HirValueRange::empty(),
                result: None,
            },
            1,
        )],
        terminator: HirTerminator::Return(unit_expression(
            types.unit,
            RegionId(0),
            &mut expressions,
        )),
    };

    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![(start_function, start_path, HirFunctionOrigin::EntryStart)],
        vec![start_block],
        FunctionId(0),
    );

    let error = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect_err("unsupported host call should produce diagnostic");
    let error = error
        .infrastructure_error()
        .expect("Wasm lowering failure should be wrapped for rendering");
    assert!(error.msg.contains("<synthetic>"));
}

#[test]
fn selected_function_policy_ignores_unselected_host_calls_and_assertions() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (mut type_environment, types) = build_type_environment();
    let option_string = type_environment.intern_option(types.string);
    let start_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");
    let unused_path = path_fork
        .try_intern_portable_path("unused", &mut string_table)
        .expect("test path fits");

    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(unit_expression(
            types.unit,
            RegionId(0),
            &mut expressions,
        )),
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
                target: CallTarget::External(unsupported_id),
                args: HirValueRange::empty(),
                result: None,
            },
            1,
        )],
        terminator: HirTerminator::AssertFailure {
            message: expression(
                HirExpressionKind::VariantConstruct {
                    carrier: HirVariantCarrier::Option,
                    variant_index: 0,
                    fields: HirVariantFieldRange::empty(),
                },
                option_string,
                RegionId(0),
                ValueKind::Const,
                &mut expressions,
            ),
            message_evaluation: HirAssertionMessageEvaluation::Default,
        },
    };

    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };
    let unused_function = HirFunction {
        id: FunctionId(1),
        entry: BlockId(10),
        params: vec![],
        return_type: types.unit,
    };
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
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
        function_emission_policy: {
            let facts =
                crate::compiler_frontend::hir::reachability::collect_module_function_link_facts(
                    &module,
                )
                .expect("test HIR should produce function link facts");
            let reachability = crate::compiler_frontend::hir::reachability::collect_reachability_from_function_link_facts(
                &facts,
                &[module.start_function.expect("normal test module should have start")],
            )
            .expect("test HIR should produce entry reachability");
            WasmFunctionEmissionPolicy::Selected(reachability.backend_selection().clone())
        },
        ..Default::default()
    };

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &request,
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("unreachable host calls and assertions should not be lowered");

    assert_eq!(result.lir_module.imports.len(), 0);
    assert_eq!(
        result.lir_module.functions.len(),
        2,
        "only the exported start function and its wrapper should be lowered"
    );

    let mut invalid_export_request = request.clone();
    invalid_export_request
        .export_policy
        .exported_functions
        .push(FunctionId(1));
    invalid_export_request
        .export_policy
        .export_names
        .insert(FunctionId(1), "unused".to_owned());

    let error = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &invalid_export_request,
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect_err("an unselected function must not become a Wasm export");
    let error = error
        .infrastructure_error()
        .expect("Wasm request validation should fail before lowering");
    assert!(error.msg.contains("absent from the selected function plan"));
}

#[test]
fn lower_type_to_abi_maps_all_hir_types_correctly() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let start_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");

    // Build a minimal module so we can construct a WasmLirLoweringContext.
    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(
            0,
            types.int,
            RegionId(0),
            &mut expressions,
        )),
    };
    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.int,
    };
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![(start_function, start_path, HirFunctionOrigin::EntryStart)],
        vec![start_block],
        FunctionId(0),
    );

    // Additional builtin types to test all ABI mappings.
    let builtins = type_environment.builtins();
    let float_id = builtins.float;
    let char_id = builtins.char;
    let range_id = builtins.range;

    let borrow_facts = default_borrow_facts();
    let path_table = path_fork.snapshot_table();
    for profile in [
        NumericProfile::STANDARD,
        NumericProfile {
            int_width: IntWidth::Bits32,
            float_precision: FloatPrecision::Bits32,
        },
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits64,
        },
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits32,
        },
    ] {
        let request = WasmBackendRequest {
            numeric_profile: profile,
            ..Default::default()
        };
        let proofs = default_numeric_proofs();
        let context = crate::backends::wasm::hir_to_lir::context::WasmLirLoweringContext::new(
            &module,
            &borrow_facts,
            &proofs,
            &request,
            &string_table,
            &path_table,
            &type_environment,
        );

        assert_eq!(
            lower_type_to_abi(&context, types.int),
            match profile.int_width {
                IntWidth::Bits32 => WasmAbiType::I32,
                IntWidth::Bits64 => WasmAbiType::I64,
            },
            "{profile}"
        );
        assert_eq!(
            lower_type_to_abi(&context, float_id),
            match profile.float_precision {
                FloatPrecision::Bits32 => WasmAbiType::F32,
                FloatPrecision::Bits64 => WasmAbiType::F64,
            },
            "{profile}"
        );
        for scalar in FixedScalar::ALL {
            let expected = match scalar {
                FixedScalar::I64 | FixedScalar::U64 => WasmAbiType::I64,
                FixedScalar::F64 => WasmAbiType::F64,
                FixedScalar::F16 | FixedScalar::F32 => WasmAbiType::F32,
                _ => WasmAbiType::I32,
            };
            assert_eq!(
                lower_type_to_abi(&context, builtin_type_ids::fixed_scalar(scalar)),
                expected,
                "{profile} {scalar:?}"
            );
        }
        assert_eq!(lower_type_to_abi(&context, types.boolean), WasmAbiType::I32);
        assert_eq!(
            lower_type_to_abi(&context, types.string),
            WasmAbiType::Handle
        );
        assert_eq!(lower_type_to_abi(&context, types.unit), WasmAbiType::Void);
        assert_eq!(lower_type_to_abi(&context, char_id), WasmAbiType::I32);
        assert_eq!(lower_type_to_abi(&context, range_id), WasmAbiType::Handle);
    }
}

#[test]
fn multi_fragment_template_produces_all_push_operations() {
    // Verifies that a runtime template with literal + handle + literal + handle
    // produces the correct sequence: NewBuffer, PushLiteral, PushHandle, PushLiteral, PushHandle, Finish.
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let runtime_path = path_fork
        .try_intern_portable_path("__moth_frag_0", &mut string_table)
        .expect("test path fits");

    // Build: "prefix" + param0 + "middle" + param1 + "suffix"
    let prefix = string_expression("prefix", types.string, RegionId(0), &mut expressions);
    let first_handle = load_local(&mut expressions, LocalId(0), types.string, RegionId(0));
    let inner_concat_1 = expression(
        HirExpressionKind::BinOp {
            left: prefix,
            op: HirBinOp::StringAppend,
            right: first_handle,
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );
    let middle = string_expression("middle", types.string, RegionId(0), &mut expressions);
    let inner_concat_2 = expression(
        HirExpressionKind::BinOp {
            left: inner_concat_1,
            op: HirBinOp::StringAppend,
            right: middle,
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );
    let second_handle = load_local(&mut expressions, LocalId(1), types.string, RegionId(0));
    let inner_concat_3 = expression(
        HirExpressionKind::BinOp {
            left: inner_concat_2,
            op: HirBinOp::StringAppend,
            right: second_handle,
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );
    let suffix = string_expression("suffix", types.string, RegionId(0), &mut expressions);
    let full_concat = expression(
        HirExpressionKind::BinOp {
            left: inner_concat_3,
            op: HirBinOp::StringAppend,
            right: suffix,
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
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
    };

    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![(runtime_function, runtime_path, HirFunctionOrigin::Normal)],
        vec![runtime_block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
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
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let fn_path = path_fork
        .try_intern_portable_path("my_helper", &mut string_table)
        .expect("test path fits");

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(
            0,
            types.int,
            RegionId(0),
            &mut expressions,
        )),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.int,
    };
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![(function, fn_path, HirFunctionOrigin::Normal)],
        vec![block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
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
    let mut path_fork = PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let (type_environment, types) = build_type_environment();
    let main_path = path_fork
        .try_intern_portable_path("main", &mut string_table)
        .expect("test path fits");

    let argument = string_expression("hello", types.string, RegionId(0), &mut expressions);
    let arguments = expressions
        .append_values(&[argument], None)
        .expect("IO call argument should fit");
    let io_call = statement(
        1,
        HirStatementKind::Call {
            target: CallTarget::External(
                crate::compiler_frontend::external_packages::ExternalFunctionId::IoLine,
            ),
            args: arguments,
            result: None,
        },
        1,
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![io_call],
        terminator: HirTerminator::Return(unit_expression(
            types.unit,
            RegionId(0),
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
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![(function, main_path, HirFunctionOrigin::EntryStart)],
        vec![block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
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
