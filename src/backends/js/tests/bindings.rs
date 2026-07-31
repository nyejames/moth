//! Local binding, alias, and computed-place JavaScript emission tests.

use super::support::*;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{
    BlockId, FieldId, FunctionId, HirNodeId, LocalId, RegionId, StructId,
};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::structs::{HirField, HirStruct};
use crate::compiler_frontend::hir::terminators::HirTerminator;

// Local binding and assignment tests [binding] [alias]
// ---------------------------------------------------------------------------

/// Verifies that assigning an integer value to a local emits __moth_assign_value. [binding]
#[test]
fn local_slot_assignment_emits_assign_value() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let assign = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: int_expression(1, 42, types.int, RegionId(0)),
        },
        1,
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![assign],
        terminator: HirTerminator::Return(unit_expression(2, types.unit, RegionId(0))),
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
        &[(LocalId(0), "count")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");
    let count_name = expected_dev_local_name("count", 0);

    assert!(
        output
            .source
            .contains(&format!("__moth_assign_value({count_name}, 42);")),
        "assigning an integer to a local must emit __moth_assign_value"
    );
}

// Parameter normalization tests [binding]
// ---------------------------------------------------------------------------

/// Verifies that function parameters emit __moth_param_binding to normalize call arguments. [binding]
#[test]
fn function_parameters_emit_param_binding() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![],
        terminator: HirTerminator::Return(unit_expression(0, types.unit, RegionId(0))),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![LocalId(0)],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "takes_arg",
        vec![block],
        function,
        &[(LocalId(0), "arg")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
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

/// Verifies that assigning a Load (borrow) to a local emits __moth_assign_borrow. [alias]
#[test]
fn borrow_assignment_emits_assign_borrow() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let assign_source = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: int_expression(1, 42, types.int, RegionId(0)),
        },
        1,
    );

    let assign_alias = statement(
        2,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(1)),
            value: expression(
                2,
                HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                types.int,
                RegionId(0),
                ValueKind::RValue,
            ),
        },
        2,
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![
            local(0, types.int, RegionId(0)),
            local(1, types.int, RegionId(0)),
        ],
        statements: vec![assign_source, assign_alias],
        terminator: HirTerminator::Return(unit_expression(3, types.unit, RegionId(0))),
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
        &[(LocalId(0), "source"), (LocalId(1), "alias")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");
    let alias_name = expected_dev_local_name("alias", 1);
    let source_name = expected_dev_local_name("source", 0);

    assert!(
        output.source.contains(&format!(
            "__moth_assign_borrow({alias_name}, {source_name})"
        )),
        "Load assignment to a fresh local must emit __moth_assign_borrow"
    );
}

/// Verifies that an alias local is read through __moth_read in a host io call. [binding]
#[test]
fn alias_local_read_emits_bs_read() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let io_id = crate::compiler_frontend::external_packages::ExternalFunctionId::IoLine;

    let assign_source = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: int_expression(1, 99, types.int, RegionId(0)),
        },
        1,
    );

    let assign_alias = statement(
        2,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(1)),
            value: expression(
                2,
                HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                types.int,
                RegionId(0),
                ValueKind::RValue,
            ),
        },
        2,
    );

    let log_alias = statement(
        3,
        HirStatementKind::Call {
            target: CallTarget::External(io_id),
            args: vec![expression(
                3,
                HirExpressionKind::Load(HirPlace::Local(LocalId(1))),
                types.int,
                RegionId(0),
                ValueKind::RValue,
            )],
            result: None,
        },
        3,
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![
            local(0, types.int, RegionId(0)),
            local(1, types.int, RegionId(0)),
        ],
        statements: vec![assign_source, assign_alias, log_alias],
        terminator: HirTerminator::Return(unit_expression(4, types.unit, RegionId(0))),
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
        &[(LocalId(0), "source"), (LocalId(1), "alias")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
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

/// Verifies that assigning to an alias-only local emits __moth_write instead of __moth_assign_value. [alias]
#[test]
fn alias_only_local_assignment_emits_write() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let assign = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: int_expression(1, 42, types.int, RegionId(0)),
        },
        1,
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![assign],
        terminator: HirTerminator::Return(unit_expression(2, types.unit, RegionId(0))),
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
        &[(LocalId(0), "target")],
    );

    // Mark the local as alias-only at the assignment statement so the emitter takes the
    // __moth_write path instead of __moth_assign_value.
    let mut report = BorrowCheckReport::default();
    report.analysis.statement_entry_states.insert(
        HirNodeId(1),
        BorrowStateSnapshot {
            locals: vec![LocalBorrowSnapshot {
                local: LocalId(0),
                mode: LocalMode::ALIAS,
                alias_roots: vec![],
            }],
        },
    );

    let output = lower_hir_to_js(
        &module,
        &report,
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");
    let target_name = expected_dev_local_name("target", 0);

    assert!(
        output
            .source
            .contains(&format!("__moth_write({target_name}, 42)")),
        "alias-only local assignment must emit __moth_write, not __moth_assign_value"
    );
    assert!(
        !output
            .source
            .contains(&format!("__moth_assign_value({target_name}")),
        "alias-only local must not use __moth_assign_value"
    );
}

// ---------------------------------------------------------------------------
// Computed-place tests [computed]
// ---------------------------------------------------------------------------

/// Verifies that assigning to a struct field emits __moth_write(__moth_field(...)). [computed]
#[test]
fn field_place_emits_bs_field() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let assign_to_field = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Field {
                base: Box::new(HirPlace::Local(LocalId(0))),
                field: FieldId(0),
            },
            value: int_expression(1, 42, types.int, RegionId(0)),
        },
        1,
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![assign_to_field],
        terminator: HirTerminator::Return(unit_expression(2, types.unit, RegionId(0))),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let mut module = build_module(
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
        InternedPath::from_single_str("x", &mut string_table),
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
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
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let assign_to_index = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Index {
                base: Box::new(HirPlace::Local(LocalId(0))),
                index: Box::new(int_expression(10, 0, types.int, RegionId(0))),
            },
            value: int_expression(1, 42, types.int, RegionId(0)),
        },
        1,
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![assign_to_index],
        terminator: HirTerminator::Return(unit_expression(2, types.unit, RegionId(0))),
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
        &[(LocalId(0), "arr")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
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
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let io_id = crate::compiler_frontend::external_packages::ExternalFunctionId::IoLine;

    let log_field = statement(
        1,
        HirStatementKind::Call {
            target: CallTarget::External(io_id),
            args: vec![expression(
                1,
                HirExpressionKind::Load(HirPlace::Field {
                    base: Box::new(HirPlace::Local(LocalId(0))),
                    field: FieldId(0),
                }),
                types.int,
                RegionId(0),
                ValueKind::RValue,
            )],
            result: None,
        },
        1,
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![log_field],
        terminator: HirTerminator::Return(unit_expression(2, types.unit, RegionId(0))),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let mut module = build_module(
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
        InternedPath::from_single_str("x", &mut string_table),
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
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
