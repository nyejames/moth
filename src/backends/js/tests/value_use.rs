//! Value-use lowering parity tests for the JavaScript backend.
//!
//! WHAT: verifies that HIR Load and Copy expressions lower correctly across all consumption
//! contexts after the value-use extraction.
//! WHY: the value-use helper is the single point of truth for Load/Copy ABI policy.

use super::support::*;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::functions::HirFunctionOrigin;
use crate::compiler_frontend::hir::ids::{
    BlockId, FieldId, FunctionId, LocalId, RegionId, StructId,
};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::structs::{HirField, HirStruct};
use crate::compiler_frontend::hir::terminators::HirTerminator;

// Plain expression contexts [value_use]
// ---------------------------------------------------------------------------

/// Verifies that ordinary expression lowering reads loads and clones copies.
#[test]
fn plain_expression_load_and_copy_use_read_and_clone() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let expression_statement = statement(
        1,
        HirStatementKind::Expr(expression(
            1,
            HirExpressionKind::TupleConstruct {
                elements: vec![
                    expression(
                        2,
                        HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                        types.string,
                        region,
                        ValueKind::RValue,
                    ),
                    expression(
                        3,
                        HirExpressionKind::Copy(HirPlace::Local(LocalId(1))),
                        types.string,
                        region,
                        ValueKind::RValue,
                    ),
                ],
            },
            types.string,
            region,
            ValueKind::RValue,
        )),
        1,
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![
            local(0, types.string, region),
            local(1, types.string, region),
        ],
        statements: vec![expression_statement],
        terminator: HirTerminator::Return(unit_expression(4, types.unit, region)),
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
        "main",
        vec![block],
        function,
        &[(LocalId(0), "loaded"), (LocalId(1), "copied")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    let loaded_name = expected_dev_local_name("loaded", 0);
    let copied_name = expected_dev_local_name("copied", 1);

    assert!(
        output.source.contains(&format!(
            "[__moth_read({loaded_name}), __moth_clone_value(__moth_read({copied_name}))];"
        )),
        "plain expression lowering must read Load and clone Copy"
    );
}

// Assignment contexts [value_use]
// ---------------------------------------------------------------------------

/// Verifies that assigning Load and Copy to non-local places emits concrete values.
#[test]
fn load_and_copy_in_nonlocal_assignment_emit_concrete_values() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let assign_source = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: string_expression(1, "hello", types.string, region),
        },
        1,
    );

    let copy_to_field = statement(
        2,
        HirStatementKind::Assign {
            target: HirPlace::Field {
                base: Box::new(HirPlace::Local(LocalId(0))),
                field: FieldId(0),
            },
            value: expression(
                2,
                HirExpressionKind::Copy(HirPlace::Local(LocalId(1))),
                types.string,
                region,
                ValueKind::RValue,
            ),
        },
        2,
    );

    let load_to_field = statement(
        3,
        HirStatementKind::Assign {
            target: HirPlace::Field {
                base: Box::new(HirPlace::Local(LocalId(0))),
                field: FieldId(1),
            },
            value: expression(
                3,
                HirExpressionKind::Load(HirPlace::Local(LocalId(2))),
                types.string,
                region,
                ValueKind::RValue,
            ),
        },
        3,
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![
            local(0, types.string, region),
            local(1, types.string, region),
            local(2, types.string, region),
        ],
        statements: vec![assign_source, copy_to_field, load_to_field],
        terminator: HirTerminator::Return(unit_expression(4, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
        return_aliases: vec![],
    };

    let mut module = build_module(
        &mut string_table,
        "main",
        vec![block],
        function,
        &[
            (LocalId(0), "target"),
            (LocalId(1), "copy_source"),
            (LocalId(2), "load_source"),
        ],
    );

    module.structs = vec![HirStruct {
        id: StructId(0),
        frontend_type_id: types.string,
        fields: vec![
            HirField {
                id: FieldId(0),
                ty: types.string,
            },
            HirField {
                id: FieldId(1),
                ty: types.string,
            },
        ],
    }];
    module.side_table.bind_field_name(
        FieldId(0),
        InternedPath::from_single_str("copy_field", &mut string_table),
    );
    module.side_table.bind_field_name(
        FieldId(1),
        InternedPath::from_single_str("load_field", &mut string_table),
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    let copy_source_name = expected_dev_local_name("copy_source", 1);
    let load_source_name = expected_dev_local_name("load_source", 2);
    let copy_field_name = expected_dev_field_name("copy_field", 0);
    let load_field_name = expected_dev_field_name("load_field", 1);
    let target_name = expected_dev_local_name("target", 0);

    assert!(
        output.source.contains(&format!(
            "__moth_write(__moth_field({target_name}, \"{copy_field_name}\"), __moth_clone_value(__moth_read({copy_source_name})))"
        )),
        "Copy in non-local assignment must emit __moth_clone_value(__moth_read(...))"
    );
    assert!(
        output.source.contains(&format!(
            "__moth_write(__moth_field({target_name}, \"{load_field_name}\"), __moth_read({load_source_name}))"
        )),
        "Load in non-local assignment must emit __moth_read(...)"
    );
}

// Moth call arguments [value_use]
// ---------------------------------------------------------------------------

/// Verifies that Moth call arguments pass loads as refs and wrap copies in bindings.
#[test]
fn load_and_copy_in_moth_call_arguments_use_reference_abi() {
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
        params: vec![LocalId(0), LocalId(2)],
        return_type: types.int,
        return_aliases: vec![None],
    };

    let call_stmt = statement(
        1,
        HirStatementKind::Call {
            target: CallTarget::Local(FunctionId(1)),
            args: vec![
                expression(
                    1,
                    HirExpressionKind::Copy(HirPlace::Local(LocalId(0))),
                    types.int,
                    region,
                    ValueKind::RValue,
                ),
                expression(
                    2,
                    HirExpressionKind::Load(HirPlace::Local(LocalId(2))),
                    types.int,
                    region,
                    ValueKind::RValue,
                ),
            ],
            result: Some(LocalId(1)),
        },
        1,
    );

    let caller_block = HirBlock {
        id: BlockId(1),
        region,
        locals: vec![
            local(0, types.int, region),
            local(1, types.int, region),
            local(2, types.int, region),
        ],
        statements: vec![call_stmt],
        terminator: HirTerminator::Return(int_expression(2, 0, types.int, region)),
    };
    let caller = HirFunction {
        id: FunctionId(0),
        entry: BlockId(1),
        params: vec![],
        return_type: types.int,
        return_aliases: vec![],
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
        InternedPath::from_single_str("callee", &mut string_table),
    );
    module.side_table.bind_local_name(
        LocalId(0),
        InternedPath::from_single_str("copy_source", &mut string_table),
    );
    module.side_table.bind_local_name(
        LocalId(1),
        InternedPath::from_single_str("result", &mut string_table),
    );
    module.side_table.bind_local_name(
        LocalId(2),
        InternedPath::from_single_str("load_source", &mut string_table),
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

    let copy_source_name = expected_dev_local_name("copy_source", 0);
    let load_source_name = expected_dev_local_name("load_source", 2);
    let callee_name = expected_dev_function_name("callee", 1);

    assert!(
        output.source.contains(&format!(
            "{callee_name}(__moth_binding(__moth_clone_value(__moth_read({copy_source_name}))), {load_source_name})"
        )),
        "Moth call arguments must wrap Copy in __moth_binding and pass Load as a ref"
    );
}

// Host call arguments [value_use]
// ---------------------------------------------------------------------------

/// Verifies that host calls receive raw values for both Load and Copy.
#[test]
fn load_and_copy_in_host_call_arguments_emit_raw_values() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let io_id = crate::compiler_frontend::external_packages::ExternalFunctionId::IoLine;

    let load_call = statement(
        1,
        HirStatementKind::Call {
            target: CallTarget::External(io_id),
            args: vec![expression(
                1,
                HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                types.string,
                region,
                ValueKind::RValue,
            )],
            result: None,
        },
        1,
    );

    let copy_call = statement(
        2,
        HirStatementKind::Call {
            target: CallTarget::External(io_id),
            args: vec![expression(
                2,
                HirExpressionKind::Copy(HirPlace::Local(LocalId(1))),
                types.string,
                region,
                ValueKind::RValue,
            )],
            result: None,
        },
        2,
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![
            local(0, types.string, region),
            local(1, types.string, region),
        ],
        statements: vec![load_call, copy_call],
        terminator: HirTerminator::Return(unit_expression(3, types.unit, region)),
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
        "main",
        vec![block],
        function,
        &[(LocalId(0), "loaded"), (LocalId(1), "copied")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    let loaded_name = expected_dev_local_name("loaded", 0);
    let copied_name = expected_dev_local_name("copied", 1);

    assert!(
        output
            .source
            .contains(&format!("__moth_io_line(__moth_read({loaded_name}))")),
        "Load in host call argument must read the raw JS value"
    );
    assert!(
        output.source.contains(&format!(
            "__moth_io_line(__moth_clone_value(__moth_read({copied_name})))"
        )),
        "Copy in host call argument must emit __moth_clone_value(__moth_read(...))"
    );
}

// Return value handling [value_use]
// ---------------------------------------------------------------------------

/// Verifies that returning a Load from an alias-returning function passes the raw place.
#[test]
fn load_in_return_value_passes_place_ref() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, types.int, region)],
        statements: vec![],
        terminator: HirTerminator::Return(expression(
            1,
            HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
            types.int,
            region,
            ValueKind::RValue,
        )),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![LocalId(0)],
        return_type: types.int,
        return_aliases: vec![Some(vec![0])],
    };

    let mut module = HirModule::new();
    module.blocks = vec![block];
    module.functions = vec![function];
    module.start_function = Some(FunctionId(0));
    module.regions = vec![HirRegion::lexical(RegionId(0), None)];
    module.side_table.bind_function_name(
        FunctionId(0),
        InternedPath::from_single_str("identity", &mut string_table),
    );
    module.side_table.bind_local_name(
        LocalId(0),
        InternedPath::from_single_str("value", &mut string_table),
    );
    module
        .function_origins
        .insert(FunctionId(0), HirFunctionOrigin::Normal);

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    let param_name = expected_dev_local_name("value", 0);

    assert!(
        output.source.contains(&format!("return {param_name};")),
        "Load in return value of alias-returning function must pass the raw place ref"
    );
    assert!(
        !output
            .source
            .contains(&format!("return __moth_read({param_name});")),
        "Load in return value must not read through __moth_read"
    );
}

/// Verifies that returning a Copy clones the value even in alias-returning context.
#[test]
fn copy_in_return_value_emits_clone_value() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, types.int, region)],
        statements: vec![],
        terminator: HirTerminator::Return(expression(
            1,
            HirExpressionKind::Copy(HirPlace::Local(LocalId(0))),
            types.int,
            region,
            ValueKind::RValue,
        )),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![LocalId(0)],
        return_type: types.int,
        return_aliases: vec![Some(vec![0])],
    };

    let mut module = HirModule::new();
    module.blocks = vec![block];
    module.functions = vec![function];
    module.start_function = Some(FunctionId(0));
    module.regions = vec![HirRegion::lexical(RegionId(0), None)];
    module.side_table.bind_function_name(
        FunctionId(0),
        InternedPath::from_single_str("clone_return", &mut string_table),
    );
    module.side_table.bind_local_name(
        LocalId(0),
        InternedPath::from_single_str("value", &mut string_table),
    );
    module
        .function_origins
        .insert(FunctionId(0), HirFunctionOrigin::Normal);

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    let param_name = expected_dev_local_name("value", 0);

    assert!(
        output.source.contains(&format!(
            "return __moth_clone_value(__moth_read({param_name}));"
        )),
        "Copy in return value must emit __moth_clone_value(__moth_read(...))"
    );
}

/// Verifies that tuple return values recursively apply return-value handling per element.
#[test]
fn tuple_return_preserves_return_value_handling_per_element() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let return_expr = expression(
        1,
        HirExpressionKind::TupleConstruct {
            elements: vec![
                expression(
                    2,
                    HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                    types.int,
                    region,
                    ValueKind::RValue,
                ),
                expression(
                    3,
                    HirExpressionKind::Copy(HirPlace::Local(LocalId(1))),
                    types.string,
                    region,
                    ValueKind::RValue,
                ),
            ],
        },
        types.int,
        region,
        ValueKind::RValue,
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, types.int, region), local(1, types.string, region)],
        statements: vec![],
        terminator: HirTerminator::Return(return_expr),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![LocalId(0), LocalId(1)],
        return_type: types.int,
        return_aliases: vec![Some(vec![0])],
    };

    let mut module = HirModule::new();
    module.blocks = vec![block];
    module.functions = vec![function];
    module.start_function = Some(FunctionId(0));
    module.regions = vec![HirRegion::lexical(RegionId(0), None)];
    module.side_table.bind_function_name(
        FunctionId(0),
        InternedPath::from_single_str("pair", &mut string_table),
    );
    module.side_table.bind_local_name(
        LocalId(0),
        InternedPath::from_single_str("first", &mut string_table),
    );
    module.side_table.bind_local_name(
        LocalId(1),
        InternedPath::from_single_str("second", &mut string_table),
    );
    module
        .function_origins
        .insert(FunctionId(0), HirFunctionOrigin::Normal);

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    let first_name = expected_dev_local_name("first", 0);
    let second_name = expected_dev_local_name("second", 1);

    assert!(
        output.source.contains(&format!(
            "return [{first_name}, __moth_clone_value(__moth_read({second_name}))];"
        )),
        "tuple return must preserve per-element return-value handling: Load as place ref, Copy as clone"
    );
}
