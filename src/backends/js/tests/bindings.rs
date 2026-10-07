//! Local binding, alias, and computed-place JavaScript emission tests.

use super::support::*;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{
    BlockId, FieldId, FunctionId, LocalId, RegionId, StructId,
};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::structs::{HirField, HirStruct};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_tests::integration_test_runner::assertions::{
    run_node_script_within, with_harness_workspace,
};
use std::time::Duration;

// Local binding and assignment tests [binding] [alias]
// ---------------------------------------------------------------------------

/// Verifies that defining a local with an integer creates a fresh value binding. [binding]
#[test]
fn value_definition_emits_fresh_binding() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();

    let assign = statement(
        1,
        HirStatementKind::Write {
            target: HirWriteTarget::DefineLocal(LocalId(0)),
            value: int_expression(42, types.int, RegionId(0), &mut expressions),
        },
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![assign],
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
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "count")],
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
    let count_name = expected_dev_local_name("count", 0);

    assert!(
        output.source.contains(&format!("let {count_name};")),
        "function setup must reserve the name before its dynamic definition"
    );
    assert!(
        !output
            .source
            .contains(&format!("let {count_name} = __moth_binding(undefined);")),
        "function setup must not allocate a placeholder binding before the definition"
    );
    assert!(
        output
            .source
            .contains(&format!("{count_name} = __moth_binding(42);")),
        "a local definition must wrap the produced value in a fresh binding"
    );
}

// Parameter normalization tests [binding]
// ---------------------------------------------------------------------------

/// Verifies that function parameters emit __moth_param_binding to normalize call arguments. [binding]
#[test]
fn function_parameters_emit_param_binding() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![],
        terminator: HirTerminator::Return(unit_expression(
            types.unit,
            RegionId(0),
            &mut expressions,
        )),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![LocalId(0)],
        return_type: types.unit,
    };

    let module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "takes_arg",
        vec![block],
        function,
        &[(LocalId(0), "arg")],
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
    let arg_name = expected_dev_local_name("arg", 0);

    assert!(
        output
            .source
            .contains(&format!("{arg_name} = __moth_param_binding({arg_name});")),
        "function parameters must be normalised through __moth_param_binding"
    );
}

// ---------------------------------------------------------------------------
// Borrow-assignment and alias behavior tests [alias]
// ---------------------------------------------------------------------------

/// Verifies that defining a local from a place creates a new alias binding. [alias]
#[test]
fn place_definition_emits_fresh_alias_binding() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();

    let assign_source = statement(
        1,
        HirStatementKind::Write {
            target: HirWriteTarget::DefineLocal(LocalId(0)),
            value: int_expression(42, types.int, RegionId(0), &mut expressions),
        },
    );

    let assign_alias = statement(
        2,
        HirStatementKind::Write {
            target: HirWriteTarget::DefineLocal(LocalId(1)),
            value: expression(
                HirExpressionKind::Load(HirPlace::local(LocalId(0))),
                types.int,
                RegionId(0),
                ValueKind::Place,
                &mut expressions,
            ),
        },
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![
            local(0, types.int, RegionId(0)),
            local(1, types.int, RegionId(0)),
        ],
        statements: vec![assign_source, assign_alias],
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
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "source"), (LocalId(1), "alias")],
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
    let alias_name = expected_dev_local_name("alias", 1);
    let source_name = expected_dev_local_name("source", 0);

    assert!(
        output.source.contains(&format!(
            "{alias_name} = __moth_alias_binding({source_name});"
        )),
        "a place-valued definition must create a fresh alias wrapper"
    );
}

/// Verifies that updating an existing local from a place uses borrow-assignment semantics. [alias]
#[test]
fn place_update_emits_assign_borrow() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();

    let update = statement(
        1,
        HirStatementKind::Write {
            target: HirWriteTarget::AssignPlace(HirPlace::local(LocalId(1))),
            value: expression(
                HirExpressionKind::Load(HirPlace::local(LocalId(0))),
                types.int,
                RegionId(0),
                ValueKind::Place,
                &mut expressions,
            ),
        },
    );
    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![
            local(0, types.int, RegionId(0)),
            local(1, types.int, RegionId(0)),
        ],
        statements: vec![update],
        terminator: HirTerminator::Return(unit_expression(
            types.unit,
            RegionId(0),
            &mut expressions,
        )),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![LocalId(0), LocalId(1)],
        return_type: types.unit,
    };
    let module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "source"), (LocalId(1), "target")],
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
    let target_name = expected_dev_local_name("target", 1);
    let source_name = expected_dev_local_name("source", 0);

    assert!(
        output.source.contains(&format!(
            "__moth_assign_borrow({target_name}, {source_name});"
        )),
        "a place-valued update must use the runtime's borrow assignment helper"
    );
}

/// Verifies that borrow-updating a slot from an alias resolving back to that slot is a no-op.
#[test]
fn place_update_from_alias_of_destination_does_not_create_cycle() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let define_alias = statement(
        1,
        HirStatementKind::Write {
            target: HirWriteTarget::DefineLocal(LocalId(1)),
            value: expression(
                HirExpressionKind::Load(HirPlace::local(LocalId(0))),
                types.int,
                RegionId(0),
                ValueKind::Place,
                &mut expressions,
            ),
        },
    );
    let update_from_alias = statement(
        2,
        HirStatementKind::Write {
            target: HirWriteTarget::AssignPlace(HirPlace::local(LocalId(0))),
            value: expression(
                HirExpressionKind::Load(HirPlace::local(LocalId(1))),
                types.int,
                RegionId(0),
                ValueKind::Place,
                &mut expressions,
            ),
        },
    );
    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![
            local(0, types.int, RegionId(0)),
            local(1, types.int, RegionId(0)),
        ],
        statements: vec![define_alias, update_from_alias],
        terminator: HirTerminator::Return(expression(
            HirExpressionKind::Load(HirPlace::local(LocalId(0))),
            types.int,
            RegionId(0),
            ValueKind::RValue,
            &mut expressions,
        )),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![LocalId(0)],
        return_type: types.int,
    };
    let module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "value"), (LocalId(1), "alias")],
    );
    let output = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("same-binding place update should lower");
    let function_name = expected_dev_function_name("main", 0);
    let script = format!(
        "{}\nconst value = __moth_binding(42);\nconsole.log({function_name}(value));",
        output.source
    );
    let stdout = with_harness_workspace(|workspace| {
        let script_path = workspace.write("program.js", &script)?;
        let run = run_node_script_within(&script_path, workspace.path(), Duration::from_secs(2))?;
        Ok(run.stdout)
    })
    .expect("a self-resolving alias update must finish under the bounded Node harness");
    assert_eq!(stdout.trim(), "42");
}

/// Verifies that an alias local is read through __moth_read in a host io call. [binding]
#[test]
fn alias_local_read_emits_bs_read() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();

    let io_id = crate::compiler_frontend::external_packages::ExternalFunctionId::IoLine;

    let assign_source = statement(
        1,
        HirStatementKind::Write {
            target: HirWriteTarget::DefineLocal(LocalId(0)),
            value: int_expression(99, types.int, RegionId(0), &mut expressions),
        },
    );

    let assign_alias = statement(
        2,
        HirStatementKind::Write {
            target: HirWriteTarget::DefineLocal(LocalId(1)),
            value: expression(
                HirExpressionKind::Load(HirPlace::local(LocalId(0))),
                types.int,
                RegionId(0),
                ValueKind::Place,
                &mut expressions,
            ),
        },
    );

    let log_alias = statement(
        3,
        HirStatementKind::Call {
            target: CallTarget::External(io_id),
            args: append_values(
                &[expression(
                    HirExpressionKind::Load(HirPlace::local(LocalId(1))),
                    types.int,
                    RegionId(0),
                    ValueKind::RValue,
                    &mut expressions,
                )],
                &mut expressions,
            ),
            result: None,
        },
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![
            local(0, types.int, RegionId(0)),
            local(1, types.int, RegionId(0)),
        ],
        statements: vec![assign_source, assign_alias, log_alias],
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
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "source"), (LocalId(1), "alias")],
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
    let alias_name = expected_dev_local_name("alias", 1);

    assert!(
        output
            .source
            .contains(&format!("__moth_io_line(__moth_read({alias_name}))")),
        "reading an alias local in a host call must go through __moth_read"
    );
}

/// Verifies that updating an existing local with a value uses value-assignment semantics. [binding]
#[test]
fn value_update_emits_assign_value() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();

    let assign = statement(
        1,
        HirStatementKind::Write {
            target: HirWriteTarget::AssignPlace(HirPlace::local(LocalId(0))),
            value: int_expression(42, types.int, RegionId(0), &mut expressions),
        },
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![assign],
        terminator: HirTerminator::Return(unit_expression(
            types.unit,
            RegionId(0),
            &mut expressions,
        )),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![LocalId(0)],
        return_type: types.unit,
    };

    let module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "target")],
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
    let target_name = expected_dev_local_name("target", 0);

    assert!(
        output
            .source
            .contains(&format!("__moth_assign_value({target_name}, 42);")),
        "an explicit local update with an rvalue must use value-assignment semantics"
    );
}

// ---------------------------------------------------------------------------
// Computed-place tests [computed]
// ---------------------------------------------------------------------------

/// Verifies that assigning to a struct field emits __moth_write(__moth_field(...)). [computed]
#[test]
fn field_place_emits_bs_field() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();

    let assign_to_field = statement(
        1,
        HirStatementKind::Write {
            target: HirWriteTarget::AssignPlace(
                HirPlace::local(LocalId(0))
                    .with_field(FieldId(0), &mut expressions, None)
                    .expect("test field projection should fit"),
            ),
            value: int_expression(42, types.int, RegionId(0), &mut expressions),
        },
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![assign_to_field],
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

    let mut module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "my_struct")],
    );

    // Register the struct and field so the field symbol map is populated.
    module.structs = vec![HirStruct {
        id: StructId(0),
        frontend_type_id: types.int,
        fields: vec![HirField {
            id: FieldId(0),
            ty: types.int,
        }],
    }];
    module.side_table.bind_field_name(
        FieldId(0),
        path_fork
            .try_intern_portable_path("x", &mut string_table)
            .expect("test path fits"),
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
    let struct_name = expected_dev_local_name("my_struct", 0);
    let field_name = expected_dev_field_name("x", 0);

    assert!(
        output.source.contains(&format!(
            "__moth_write(__moth_field({struct_name}, \"{field_name}\"), 42)"
        )),
        "field assignment must route through __moth_field and __moth_write"
    );
}

/// Verifies that assigning to a collection index emits __moth_write(__moth_index(...)). [computed]
#[test]
fn index_place_emits_bs_index() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();

    let assign_to_index = statement(
        1,
        HirStatementKind::Write {
            target: HirWriteTarget::AssignPlace({
                let index = int_expression(0, types.int, RegionId(0), &mut expressions);
                HirPlace::local(LocalId(0))
                    .with_index(index, &mut expressions, None)
                    .expect("test index projection should fit")
            }),
            value: int_expression(42, types.int, RegionId(0), &mut expressions),
        },
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![assign_to_index],
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
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "arr")],
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
    let array_name = expected_dev_local_name("arr", 0);

    assert!(
        output
            .source
            .contains(&format!("__moth_write(__moth_index({array_name}, 0), 42)")),
        "index assignment must route through __moth_index and __moth_write"
    );
}

/// Verifies that reading a field place composes __moth_read with __moth_field. [computed]
#[test]
fn computed_place_read_composes_with_bs_read() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();

    let io_id = crate::compiler_frontend::external_packages::ExternalFunctionId::IoLine;

    let log_field = statement(
        1,
        HirStatementKind::Call {
            target: CallTarget::External(io_id),
            args: {
                let field_place = HirPlace::local(LocalId(0))
                    .with_field(FieldId(0), &mut expressions, None)
                    .expect("test field projection should fit");
                let field_value = expression(
                    HirExpressionKind::Load(field_place),
                    types.int,
                    RegionId(0),
                    ValueKind::RValue,
                    &mut expressions,
                );
                append_values(&[field_value], &mut expressions)
            },
            result: None,
        },
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![log_field],
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

    let mut module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "my_struct")],
    );

    module.structs = vec![HirStruct {
        id: StructId(0),
        frontend_type_id: types.int,
        fields: vec![HirField {
            id: FieldId(0),
            ty: types.int,
        }],
    }];
    module.side_table.bind_field_name(
        FieldId(0),
        path_fork
            .try_intern_portable_path("x", &mut string_table)
            .expect("test path fits"),
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
    let struct_name = expected_dev_local_name("my_struct", 0);
    let field_name = expected_dev_field_name("x", 0);

    assert!(
        output.source.contains(&format!(
            "__moth_read(__moth_field({struct_name}, \"{field_name}\"))"
        )),
        "field Load must compose __moth_read around __moth_field"
    );
}

// ---------------------------------------------------------------------------
