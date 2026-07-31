//! Map operation statement lowering tests for JavaScript output.

use super::support::*;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, HirMapOp, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;

// Map operation statement lowering tests [map]
// ---------------------------------------------------------------------------

/// Verifies that a map `get` statement lowers to `__moth_map_get` with receiver and key. [map]
#[test]
fn map_get_statement_lowers_to_helper() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let receiver = expression(
        1,
        HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
        types.map_string_int,
        region,
        ValueKind::Place,
    );
    let key = string_expression(2, "Priya", types.string, region);

    let get_stmt = statement(
        1,
        HirStatementKind::MapOp {
            op: HirMapOp::Get,
            receiver,
            args: vec![key],
            result: Some(LocalId(1)),
        },
        1,
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![
            local(0, types.map_string_int, region),
            local(1, types.int, region),
        ],
        statements: vec![get_stmt],
        terminator: HirTerminator::Return(unit_expression(3, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "map"), (LocalId(1), "result")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(
        output
            .source
            .contains("__moth_map_get(__moth_read(moth_map_l0), \"Priya\")"),
        "map get must lower to __moth_map_get helper"
    );
    assert!(
        output
            .source
            .contains("__moth_assign_value(moth_result_l1, __moth_map_get"),
        "map get with result local must assign via __moth_assign_value"
    );
}

/// Verifies that a map `set` statement without a result local emits a plain call. [map]
#[test]
fn map_set_statement_without_result_emits_plain_call() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let receiver = expression(
        1,
        HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
        types.map_string_int,
        region,
        ValueKind::Place,
    );
    let key = string_expression(2, "Priya", types.string, region);
    let value = int_expression(3, 42, types.int, region);

    let set_stmt = statement(
        1,
        HirStatementKind::MapOp {
            op: HirMapOp::Set,
            receiver,
            args: vec![key, value],
            result: None,
        },
        1,
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, types.map_string_int, region)],
        statements: vec![set_stmt],
        terminator: HirTerminator::Return(unit_expression(4, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "map")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(
        output
            .source
            .contains("__moth_map_set(__moth_read(moth_map_l0), \"Priya\", 42);"),
        "map set without result must emit plain helper call"
    );
}

/// Verifies that map `contains`, `clear`, and `length` lower to their helpers. [map]
#[test]
fn map_infallible_ops_lower_to_plain_helpers() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let receiver = expression(
        1,
        HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
        types.map_string_int,
        region,
        ValueKind::Place,
    );
    let key = string_expression(2, "Priya", types.string, region);

    let contains_stmt = statement(
        1,
        HirStatementKind::MapOp {
            op: HirMapOp::Contains,
            receiver: receiver.clone(),
            args: vec![key.clone()],
            result: Some(LocalId(1)),
        },
        1,
    );

    let clear_stmt = statement(
        2,
        HirStatementKind::MapOp {
            op: HirMapOp::Clear,
            receiver: receiver.clone(),
            args: vec![],
            result: None,
        },
        2,
    );

    let length_stmt = statement(
        3,
        HirStatementKind::MapOp {
            op: HirMapOp::Length,
            receiver,
            args: vec![],
            result: Some(LocalId(2)),
        },
        3,
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![
            local(0, types.map_string_int, region),
            local(1, types.boolean, region),
            local(2, types.int, region),
        ],
        statements: vec![contains_stmt, clear_stmt, length_stmt],
        terminator: HirTerminator::Return(unit_expression(4, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "main",
        vec![block],
        function,
        &[
            (LocalId(0), "map"),
            (LocalId(1), "has_it"),
            (LocalId(2), "count"),
        ],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(
        output
            .source
            .contains("__moth_map_contains(__moth_read(moth_map_l0), \"Priya\")"),
        "map contains must lower to __moth_map_contains"
    );
    assert!(
        output
            .source
            .contains("__moth_map_clear(__moth_read(moth_map_l0))"),
        "map clear must lower to __moth_map_clear"
    );
    assert!(
        output
            .source
            .contains("__moth_map_length(__moth_read(moth_map_l0))"),
        "map length must lower to __moth_map_length"
    );
}

/// Verifies that a map `remove` statement lowers to `__moth_map_remove` with receiver and key. [map]
#[test]
fn map_remove_statement_lowers_to_helper() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let receiver = expression(
        1,
        HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
        types.map_string_int,
        region,
        ValueKind::Place,
    );
    let key = string_expression(2, "Priya", types.string, region);

    let remove_stmt = statement(
        1,
        HirStatementKind::MapOp {
            op: HirMapOp::Remove,
            receiver,
            args: vec![key],
            result: Some(LocalId(1)),
        },
        1,
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![
            local(0, types.map_string_int, region),
            local(1, types.int, region),
        ],
        statements: vec![remove_stmt],
        terminator: HirTerminator::Return(unit_expression(3, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "map"), (LocalId(1), "removed")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(
        output
            .source
            .contains("__moth_map_remove(__moth_read(moth_map_l0), \"Priya\")"),
        "map remove must lower to __moth_map_remove helper"
    );
    assert!(
        output
            .source
            .contains("__moth_assign_value(moth_removed_l1, __moth_map_remove"),
        "map remove with result local must assign via __moth_assign_value"
    );
}
