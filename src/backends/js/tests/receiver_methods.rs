//! Receiver-method call emission tests for JavaScript output.

use super::support::*;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

// Receiver method call emission tests [receiver]
// ---------------------------------------------------------------------------

/// Verifies that a receiver method call passes the receiver binding as the first argument. [receiver]
#[test]
fn receiver_method_call_emits_receiver_as_first_arg() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    // Callee: bump |this Int| -> Int { return this + 1 }
    let callee_block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(42, types.int, region, &mut expressions)),
    };
    let callee = HirFunction {
        id: FunctionId(1),
        entry: BlockId(0),
        params: vec![LocalId(0)],
        return_type: types.int,
    };

    // Caller: let receiver = 7; let result = bump(receiver); return result
    let assign_receiver = statement(
        1,
        HirStatementKind::Write {
            target: HirWriteTarget::DefineLocal(LocalId(0)),
            value: int_expression(7, types.int, region, &mut expressions),
        },
    );
    let call_bump = statement(
        2,
        HirStatementKind::Call {
            target: CallTarget::Local(FunctionId(1)),
            args: append_values(
                &[expression(
                    HirExpressionKind::Load(HirPlace::local(LocalId(0))),
                    types.int,
                    region,
                    ValueKind::Place,
                    &mut expressions,
                )],
                &mut expressions,
            ),
            result: Some(HirLocalDestination::Define(LocalId(1))),
        },
    );
    let caller_block = HirBlock {
        id: BlockId(1),
        region,
        locals: vec![local(0, types.int, region), local(1, types.int, region)],
        statements: vec![assign_receiver, call_bump],
        terminator: HirTerminator::Return(expression(
            HirExpressionKind::Load(HirPlace::local(LocalId(1))),
            types.int,
            region,
            ValueKind::RValue,
            &mut expressions,
        )),
    };
    let caller = HirFunction {
        id: FunctionId(0),
        entry: BlockId(1),
        params: vec![],
        return_type: types.int,
    };

    let mut module = HirModule::new();
    expressions.freeze();
    module.expressions = expressions;
    module.blocks = vec![callee_block, caller_block];
    module.functions = vec![caller, callee];
    module.start_function = Some(FunctionId(0));
    module.regions = vec![HirRegion::lexical(RegionId(0), None)];
    module.side_table.bind_function_name(
        FunctionId(0),
        path_fork
            .try_intern_portable_path("main", &mut string_table)
            .expect("test path fits"),
    );
    module.side_table.bind_function_name(
        FunctionId(1),
        path_fork
            .try_intern_portable_path("bump", &mut string_table)
            .expect("test path fits"),
    );
    module.side_table.bind_local_name(
        LocalId(0),
        path_fork
            .try_intern_portable_path("receiver", &mut string_table)
            .expect("test path fits"),
    );
    module.side_table.bind_local_name(
        LocalId(1),
        path_fork
            .try_intern_portable_path("result", &mut string_table)
            .expect("test path fits"),
    );
    module
        .function_origins
        .insert(FunctionId(0), HirFunctionOrigin::Normal);
    module
        .function_origins
        .insert(FunctionId(1), HirFunctionOrigin::Normal);

    let output = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("JS lowering should succeed");

    let receiver_name = expected_dev_local_name("receiver", 0);
    let callee_name = expected_dev_function_name("bump", 1);

    assert!(
        output
            .source
            .contains(&format!("{callee_name}({receiver_name})")),
        "receiver method call must pass receiver binding as first argument"
    );
}

/// Verifies that a receiver method return creates a fresh value-backed result binding. [receiver] [alias]
#[test]
fn receiver_method_call_assigns_value_for_return() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let callee_block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(42, types.int, region, &mut expressions)),
    };
    let callee = HirFunction {
        id: FunctionId(1),
        entry: BlockId(0),
        params: vec![LocalId(0)],
        return_type: types.int,
    };

    let call_bump = statement(
        1,
        HirStatementKind::Call {
            target: CallTarget::Local(FunctionId(1)),
            args: append_values(
                &[expression(
                    HirExpressionKind::Load(HirPlace::local(LocalId(0))),
                    types.int,
                    region,
                    ValueKind::Place,
                    &mut expressions,
                )],
                &mut expressions,
            ),
            result: Some(HirLocalDestination::Define(LocalId(1))),
        },
    );
    let caller_block = HirBlock {
        id: BlockId(1),
        region,
        locals: vec![local(0, types.int, region), local(1, types.int, region)],
        statements: vec![call_bump],
        terminator: HirTerminator::Return(int_expression(0, types.int, region, &mut expressions)),
    };
    let caller = HirFunction {
        id: FunctionId(0),
        entry: BlockId(1),
        params: vec![],
        return_type: types.int,
    };

    let mut module = HirModule::new();
    expressions.freeze();
    module.expressions = expressions;
    module.blocks = vec![callee_block, caller_block];
    module.functions = vec![caller, callee];
    module.start_function = Some(FunctionId(0));
    module.regions = vec![HirRegion::lexical(RegionId(0), None)];
    module.side_table.bind_function_name(
        FunctionId(0),
        path_fork
            .try_intern_portable_path("main", &mut string_table)
            .expect("test path fits"),
    );
    module.side_table.bind_function_name(
        FunctionId(1),
        path_fork
            .try_intern_portable_path("bump", &mut string_table)
            .expect("test path fits"),
    );
    module.side_table.bind_local_name(
        LocalId(0),
        path_fork
            .try_intern_portable_path("receiver", &mut string_table)
            .expect("test path fits"),
    );
    module.side_table.bind_local_name(
        LocalId(1),
        path_fork
            .try_intern_portable_path("result", &mut string_table)
            .expect("test path fits"),
    );
    module
        .function_origins
        .insert(FunctionId(0), HirFunctionOrigin::Normal);
    module
        .function_origins
        .insert(FunctionId(1), HirFunctionOrigin::Normal);

    let output = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("JS lowering should succeed");

    let result_name = expected_dev_local_name("result", 1);
    let callee_name = expected_dev_function_name("bump", 1);

    assert!(
        output
            .source
            .contains(&format!("{result_name} = __moth_binding({callee_name}(")),
        "moth-return receiver call must install its fresh result binding"
    );
}

// ---------------------------------------------------------------------------
