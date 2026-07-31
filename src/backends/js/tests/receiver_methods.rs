//! Receiver-method call emission tests for JavaScript output.

use super::support::*;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;

// Receiver method call emission tests [receiver]
// ---------------------------------------------------------------------------

/// Verifies that a receiver method call passes the receiver binding as the first argument. [receiver]
#[test]
fn receiver_method_call_emits_receiver_as_first_arg() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    // Callee: bump |this Int| -> Int { return this + 1 }
    let callee_block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(1, 42, types.int, region)),
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
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: int_expression(1, 7, types.int, region),
        },
        1,
    );
    let call_bump = statement(
        2,
        HirStatementKind::Call {
            target: CallTarget::Local(FunctionId(1)),
            args: vec![expression(
                2,
                HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                types.int,
                region,
                ValueKind::Place,
            )],
            result: Some(LocalId(1)),
        },
        2,
    );
    let caller_block = HirBlock {
        id: BlockId(1),
        region,
        locals: vec![local(0, types.int, region), local(1, types.int, region)],
        statements: vec![assign_receiver, call_bump],
        terminator: HirTerminator::Return(expression(
            3,
            HirExpressionKind::Load(HirPlace::Local(LocalId(1))),
            types.int,
            region,
            ValueKind::RValue,
        )),
    };
    let caller = HirFunction {
        id: FunctionId(0),
        entry: BlockId(1),
        params: vec![],
        return_type: types.int,
    };

    let mut module = HirModule::new();
    module.blocks = vec![callee_block, caller_block];
    module.functions = vec![caller, callee];
    module.start_function = Some(FunctionId(0));
    module.regions = vec![HirRegion::lexical(RegionId(0), None)];
    module.side_table.bind_function_name(
        FunctionId(0),
        InternedPath::from_single_str("main", &mut string_table),
    );
    module.side_table.bind_function_name(
        FunctionId(1),
        InternedPath::from_single_str("bump", &mut string_table),
    );
    module.side_table.bind_local_name(
        LocalId(0),
        InternedPath::from_single_str("receiver", &mut string_table),
    );
    module.side_table.bind_local_name(
        LocalId(1),
        InternedPath::from_single_str("result", &mut string_table),
    );
    module
        .function_origins
        .insert(FunctionId(0), HirFunctionOrigin::Normal);
    module
        .function_origins
        .insert(FunctionId(1), HirFunctionOrigin::Normal);

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
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

/// Verifies that a receiver method with a fresh return emits __moth_assign_value. [receiver] [alias]
#[test]
fn receiver_method_call_assigns_value_for_fresh_return() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let callee_block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(int_expression(1, 42, types.int, region)),
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
            args: vec![expression(
                1,
                HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                types.int,
                region,
                ValueKind::Place,
            )],
            result: Some(LocalId(1)),
        },
        1,
    );
    let caller_block = HirBlock {
        id: BlockId(1),
        region,
        locals: vec![local(0, types.int, region), local(1, types.int, region)],
        statements: vec![call_bump],
        terminator: HirTerminator::Return(int_expression(2, 0, types.int, region)),
    };
    let caller = HirFunction {
        id: FunctionId(0),
        entry: BlockId(1),
        params: vec![],
        return_type: types.int,
    };

    let mut module = HirModule::new();
    module.blocks = vec![callee_block, caller_block];
    module.functions = vec![caller, callee];
    module.start_function = Some(FunctionId(0));
    module.regions = vec![HirRegion::lexical(RegionId(0), None)];
    module.side_table.bind_function_name(
        FunctionId(0),
        InternedPath::from_single_str("main", &mut string_table),
    );
    module.side_table.bind_function_name(
        FunctionId(1),
        InternedPath::from_single_str("bump", &mut string_table),
    );
    module.side_table.bind_local_name(
        LocalId(0),
        InternedPath::from_single_str("receiver", &mut string_table),
    );
    module.side_table.bind_local_name(
        LocalId(1),
        InternedPath::from_single_str("result", &mut string_table),
    );
    module
        .function_origins
        .insert(FunctionId(0), HirFunctionOrigin::Normal);
    module
        .function_origins
        .insert(FunctionId(1), HirFunctionOrigin::Normal);

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    let result_name = expected_dev_local_name("result", 1);
    let callee_name = expected_dev_function_name("bump", 1);

    assert!(
        output.source.contains(&format!(
            "__moth_assign_value({result_name}, {callee_name}("
        )),
        "fresh-return receiver call must assign result with __moth_assign_value"
    );
}

// ---------------------------------------------------------------------------
