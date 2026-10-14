//! Native map operation channel lowering tests for JavaScript output.

use super::support::*;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, HirMapOp, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId, LocalId, RegionId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirLocalDestination, HirStatementKind};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};

// Map operation channel lowering tests [map]
// ---------------------------------------------------------------------------

/// Verifies that map `get` invokes its helper once and binds the success value. [map]
#[test]
fn map_get_invoke_lowers_to_helper() {
    let mut expressions = HirExpressionStore::default();
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let receiver = expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(0))),
        types.map_string_i32,
        region,
        ValueKind::Place,
        &mut expressions,
    );
    let key = string_expression("Priya", types.string, region, &mut expressions);

    let operation = HirFallibleOperation::MapOp {
        op: HirMapOp::Get,
        receiver,
        args: append_values(&[key], &mut expressions),
    };

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![
            local(0, types.map_string_i32, region),
            local(1, types.i32, region),
            local(2, types.error, region),
        ],
        statements: vec![],
        terminator: HirTerminator::Invoke {
            id: HirNodeId(1),
            span: None,
            operation,
            success: BlockId(1),
            success_results: vec![LocalId(1)],
            failure: BlockId(2),
            error_result: LocalId(2),
        },
    };
    let [success, failure] = map_continuations(&mut expressions, types.error, region);

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        returns: (vec![]).into_boxed_slice(),
        error_return: Some(types.error),
    };

    let module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block, success, failure],
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

    let result_name = invoke_result_name(&output.source);
    assert_eq!(
        output
            .source
            .matches("__moth_map_get(__moth_read(moth_map_l0), \"Priya\")")
            .count(),
        1,
        "map get must execute its helper exactly once"
    );
    assert!(
        output.source.contains(&format!(
            "moth_result_l1 = __moth_binding({result_name}.value)"
        )),
        "map get success definition must create a fresh value binding"
    );
}

/// Verifies that map `set` checks its error channel without producing success locals. [map]
#[test]
fn map_set_invoke_without_success_results_checks_error_channel() {
    let mut expressions = HirExpressionStore::default();
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let receiver = expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(0))),
        types.map_string_i32,
        region,
        ValueKind::Place,
        &mut expressions,
    );
    let key = string_expression("Priya", types.string, region, &mut expressions);
    let value = i32_expression(42, types.i32, region, &mut expressions);

    let operation = HirFallibleOperation::MapOp {
        op: HirMapOp::Set,
        receiver,
        args: append_values(&[key, value], &mut expressions),
    };

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![
            local(0, types.map_string_i32, region),
            local(2, types.error, region),
        ],
        statements: vec![],
        terminator: HirTerminator::Invoke {
            id: HirNodeId(1),
            span: None,
            operation,
            success: BlockId(1),
            success_results: vec![],
            failure: BlockId(2),
            error_result: LocalId(2),
        },
    };
    let [success, failure] = map_continuations(&mut expressions, types.error, region);

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        returns: (vec![]).into_boxed_slice(),
        error_return: Some(types.error),
    };

    let module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block, success, failure],
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

    let result_name = invoke_result_name(&output.source);
    assert_eq!(
        output
            .source
            .matches("__moth_map_set(__moth_read(moth_map_l0), \"Priya\", 42)")
            .count(),
        1,
        "map set must execute its helper exactly once"
    );
    assert!(
        output
            .source
            .contains(&format!("if ({result_name}.tag === \"ok\")"))
    );
    assert!(
        output
            .source
            .contains(&format!("__moth_binding({result_name}.value)"))
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
        types.map_string_i32,
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
            local(0, types.map_string_i32, region),
            local(1, types.boolean, region),
            local(2, types.i32, region),
        ],
        statements: vec![contains_stmt, clear_stmt, length_stmt],
        terminator: HirTerminator::Return(HirValueRange::empty()),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        returns: (vec![]).into_boxed_slice(),
        error_return: None,
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

/// Verifies that map `remove` invokes its helper and binds only its success value. [map]
#[test]
fn map_remove_invoke_lowers_to_helper() {
    let mut expressions = HirExpressionStore::default();
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let receiver = expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(0))),
        types.map_string_i32,
        region,
        ValueKind::Place,
        &mut expressions,
    );
    let key = string_expression("Priya", types.string, region, &mut expressions);

    let operation = HirFallibleOperation::MapOp {
        op: HirMapOp::Remove,
        receiver,
        args: append_values(&[key], &mut expressions),
    };

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![
            local(0, types.map_string_i32, region),
            local(1, types.i32, region),
            local(2, types.error, region),
        ],
        statements: vec![],
        terminator: HirTerminator::Invoke {
            id: HirNodeId(1),
            span: None,
            operation,
            success: BlockId(1),
            success_results: vec![LocalId(1)],
            failure: BlockId(2),
            error_result: LocalId(2),
        },
    };
    let [success, failure] = map_continuations(&mut expressions, types.error, region);

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        returns: (vec![]).into_boxed_slice(),
        error_return: Some(types.error),
    };

    let module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block, success, failure],
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

    let result_name = invoke_result_name(&output.source);
    assert_eq!(
        output
            .source
            .matches("__moth_map_remove(__moth_read(moth_map_l0), \"Priya\")")
            .count(),
        1,
        "map remove must execute its helper exactly once"
    );
    assert!(
        output.source.contains(&format!(
            "moth_removed_l1 = __moth_binding({result_name}.value)"
        )),
        "map remove success must install a fresh value binding"
    );
}

/// Gives helper-demand fixtures genuine mutually exclusive success and error exits.
fn map_continuations(
    expressions: &mut HirExpressionStore,
    error_type: TypeId,
    region: RegionId,
) -> [HirBlock; 2] {
    [
        HirBlock {
            id: BlockId(1),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(HirValueRange::empty()),
        },
        HirBlock {
            id: BlockId(2),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::ReturnError(expression(
                HirExpressionKind::Load(HirPlace::local(LocalId(2))),
                error_type,
                region,
                ValueKind::RValue,
                expressions,
            )),
        },
    ]
}

fn invoke_result_name(source: &str) -> &str {
    source
        .lines()
        .find_map(|line| {
            let declaration = line.trim().strip_prefix("const ")?;
            let (name, _) = declaration.split_once(" = ")?;
            name.starts_with("__invoke_result_").then_some(name)
        })
        .expect("map Invoke must capture its ABI result exactly once")
}
