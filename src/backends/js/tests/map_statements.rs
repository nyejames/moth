//! Map operation statement lowering tests for JavaScript output.

use super::support::*;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, HirMapOp, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirLocalDestination, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;

// Map operation statement lowering tests [map]
// ---------------------------------------------------------------------------

/// Verifies that a map `get` statement lowers to `__moth_map_get` with receiver and key. [map]
#[test]
fn map_get_statement_lowers_to_helper() {
    let mut expressions = HirExpressionStore::default();
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let receiver = expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(0))),
        types.map_string_int,
        region,
        ValueKind::Place,
        &mut expressions,
    );
    let key = string_expression("Priya", types.string, region, &mut expressions);

    let get_stmt = statement(
        1,
        HirStatementKind::MapOp {
            op: HirMapOp::Get,
            receiver,
            args: append_values(&[key], &mut expressions),
            result: Some(HirLocalDestination::Define(LocalId(1))),
        },
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![
            local(0, types.map_string_int, region),
            local(1, types.int, region),
        ],
        statements: vec![get_stmt],
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
        &[(LocalId(0), "map"), (LocalId(1), "result")],
    );

    let output = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
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
            .contains("moth_result_l1 = __moth_binding(__moth_map_get"),
        "map get result definition must create a fresh value binding"
    );
}

/// Verifies that a map `set` statement without a result local emits a plain call. [map]
#[test]
fn map_set_statement_without_result_emits_plain_call() {
    let mut expressions = HirExpressionStore::default();
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let receiver = expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(0))),
        types.map_string_int,
        region,
        ValueKind::Place,
        &mut expressions,
    );
    let key = string_expression("Priya", types.string, region, &mut expressions);
    let value = int_expression(42, types.int, region, &mut expressions);

    let set_stmt = statement(
        1,
        HirStatementKind::MapOp {
            op: HirMapOp::Set,
            receiver,
            args: append_values(&[key, value], &mut expressions),
            result: None,
        },
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, types.map_string_int, region)],
        statements: vec![set_stmt],
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
        &[(LocalId(0), "map")],
    );

    let output = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
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
    let mut expressions = HirExpressionStore::default();
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let receiver = expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(0))),
        types.map_string_int,
        region,
        ValueKind::Place,
        &mut expressions,
    );
    let key = string_expression("Priya", types.string, region, &mut expressions);

    let contains_stmt = statement(
        1,
        HirStatementKind::MapOp {
            op: HirMapOp::Contains,
            receiver,
            args: append_values(&[key], &mut expressions),
            result: Some(HirLocalDestination::Define(LocalId(1))),
        },
    );

    let clear_stmt = statement(
        2,
        HirStatementKind::MapOp {
            op: HirMapOp::Clear,
            receiver,
            args: append_values(&[], &mut expressions),
            result: None,
        },
    );

    let length_stmt = statement(
        3,
        HirStatementKind::MapOp {
            op: HirMapOp::Length,
            receiver,
            args: append_values(&[], &mut expressions),
            result: Some(HirLocalDestination::Define(LocalId(2))),
        },
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
        &[
            (LocalId(0), "map"),
            (LocalId(1), "has_it"),
            (LocalId(2), "count"),
        ],
    );

    let output = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
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
    let mut expressions = HirExpressionStore::default();
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let receiver = expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(0))),
        types.map_string_int,
        region,
        ValueKind::Place,
        &mut expressions,
    );
    let key = string_expression("Priya", types.string, region, &mut expressions);

    let remove_stmt = statement(
        1,
        HirStatementKind::MapOp {
            op: HirMapOp::Remove,
            receiver,
            args: append_values(&[key], &mut expressions),
            result: Some(HirLocalDestination::Define(LocalId(1))),
        },
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![
            local(0, types.map_string_int, region),
            local(1, types.int, region),
        ],
        statements: vec![remove_stmt],
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
        &[(LocalId(0), "map"), (LocalId(1), "removed")],
    );

    let output = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
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
            .contains("moth_removed_l1 = __moth_binding(__moth_map_remove"),
        "map remove with a defined result local must install a fresh value binding"
    );
}
