//! JavaScript lowering tests for assertion failure messages.
//!
//! WHAT: pins one-time failure-edge evaluation and optional-message selection for both JS CFG
//!       lowering strategies.
//! WHY: assertion messages are ordinary HIR values, so the backend must lower the value exactly
//!      once at the failure terminator without eagerly evaluating successful assertions.

use super::support::*;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{
    HirExpressionKind, HirMapEntry, HirVariantCarrier, HirVariantField, ValueKind,
};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::reactivity::{
    HirReactiveSource, HirReactiveSourceKind, HirReactiveTemplate, HirReactiveTemplateDependency,
    ReactiveSourceId, ReactiveTemplateId,
};
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::{HirAssertionMessageEvaluation, HirTerminator};
use crate::compiler_frontend::symbols::interned_path::InternedPath;

fn optional_message(
    id: u32,
    type_id: crate::compiler_frontend::datatypes::ids::TypeId,
    inner: crate::compiler_frontend::hir::expressions::HirExpression,
    variant_index: usize,
    value_kind: ValueKind,
) -> crate::compiler_frontend::hir::expressions::HirExpression {
    expression(
        id,
        HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Option,
            variant_index,
            fields: if variant_index == 0 {
                vec![]
            } else {
                vec![HirVariantField {
                    name: None,
                    value: inner,
                }]
            },
        },
        type_id,
        RegionId(0),
        value_kind,
    )
}

fn function_with_assertion(
    blocks: Vec<HirBlock>,
    string_table: &mut StringTable,
    type_environment: &crate::compiler_frontend::datatypes::environment::TypeEnvironment,
    types: &TypeIds,
    function_name: &str,
    local_names: &[(LocalId, &str)],
) -> String {
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };
    let module = build_module(string_table, function_name, blocks, function, local_names);

    lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        string_table,
        default_config(),
        type_environment,
    )
    .expect("JS assertion lowering should succeed")
    .source
}

#[test]
fn structured_assertion_message_is_lowered_once_and_selected() {
    let mut string_table = StringTable::new();
    let (mut type_environment, types) = build_type_environment();
    let option_string = type_environment.intern_option(types.string);
    let casted_message = expression(
        2,
        HirExpressionKind::Cast {
            source: Box::new(expression(
                1,
                HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                types.int,
                RegionId(0),
                ValueKind::RValue,
            )),
            policy: BuiltinCastPolicyId::IntToString,
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
    );
    let message = optional_message(3, option_string, casted_message, 1, ValueKind::RValue);
    let source = function_with_assertion(
        vec![HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![local(0, types.int, RegionId(0))],
            statements: vec![statement(
                4,
                HirStatementKind::Assign {
                    target: HirPlace::Local(LocalId(0)),
                    value: int_expression(5, 42, types.int, RegionId(0)),
                },
                1,
            )],
            terminator: HirTerminator::AssertFailure {
                message,
                message_evaluation: HirAssertionMessageEvaluation::Runtime,
            },
        }],
        &mut string_table,
        &type_environment,
        &types,
        "structured_assertion",
        &[(LocalId(0), "value")],
    );

    assert_eq!(source.matches("let __assert_message_").count(), 1);
    assert_eq!(
        source.matches("throw new Error((__assert_message_").count(),
        1
    );
    assert_eq!(
        source
            .matches("function __moth_cast_int_to_string(")
            .count(),
        1
    );
    assert_eq!(
        source
            .matches("__moth_cast_int_to_string(__moth_read(")
            .count(),
        1
    );
}

#[test]
fn dispatcher_assertion_message_is_lowered_once() {
    let mut string_table = StringTable::new();
    let (mut type_environment, types) = build_type_environment();
    let option_string = type_environment.intern_option(types.string);
    let casted_message = expression(
        5,
        HirExpressionKind::Cast {
            source: Box::new(expression(
                4,
                HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                types.boolean,
                RegionId(0),
                ValueKind::RValue,
            )),
            policy: BuiltinCastPolicyId::BoolToString,
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
    );
    let message = optional_message(6, option_string, casted_message, 1, ValueKind::RValue);
    let source = function_with_assertion(
        vec![
            HirBlock {
                id: BlockId(0),
                region: RegionId(0),
                locals: vec![local(0, types.boolean, RegionId(0))],
                statements: vec![statement(
                    7,
                    HirStatementKind::Assign {
                        target: HirPlace::Local(LocalId(0)),
                        value: bool_expression(8, true, types.boolean, RegionId(0)),
                    },
                    1,
                )],
                terminator: HirTerminator::If {
                    condition: bool_expression(1, true, types.boolean, RegionId(0)),
                    then_block: BlockId(1),
                    else_block: BlockId(2),
                },
            },
            HirBlock {
                id: BlockId(1),
                region: RegionId(0),
                locals: vec![],
                statements: vec![],
                terminator: HirTerminator::Jump {
                    target: BlockId(0),
                    args: vec![],
                },
            },
            HirBlock {
                id: BlockId(2),
                region: RegionId(0),
                locals: vec![],
                statements: vec![],
                terminator: HirTerminator::AssertFailure {
                    message,
                    message_evaluation: HirAssertionMessageEvaluation::Runtime,
                },
            },
        ],
        &mut string_table,
        &type_environment,
        &types,
        "dispatcher_assertion",
        &[(LocalId(0), "flag")],
    );

    assert!(source.contains("switch (__bb"));
    assert_eq!(source.matches("let __assert_message_").count(), 1);
    assert_eq!(
        source.matches("throw new Error((__assert_message_").count(),
        1
    );
    assert_eq!(
        source
            .matches("function __moth_cast_bool_to_string(")
            .count(),
        1
    );
    assert_eq!(
        source
            .matches("__moth_cast_bool_to_string(__moth_read(")
            .count(),
        1
    );
}

#[test]
fn default_assertion_message_skips_optional_lowering() {
    let mut string_table = StringTable::new();
    let (mut type_environment, types) = build_type_environment();
    let option_string = type_environment.intern_option(types.string);
    let message = optional_message(
        2,
        option_string,
        string_expression(1, "unused message", types.string, RegionId(0)),
        0,
        ValueKind::Const,
    );
    let source = function_with_assertion(
        vec![HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::AssertFailure {
                message,
                message_evaluation: HirAssertionMessageEvaluation::Default,
            },
        }],
        &mut string_table,
        &type_environment,
        &types,
        "default_assertion",
        &[],
    );

    assert!(source.contains("throw new Error(\"assertion failed\");"));
    assert!(!source.contains("__assert_message_"));
    assert!(!source.contains("unused message"));
}

#[test]
fn assertion_message_map_metadata_emits_map_helpers() {
    let mut string_table = StringTable::new();
    let (mut type_environment, types) = build_type_environment();
    let option_string = type_environment.intern_option(types.string);
    // This synthetic nested shape exercises the backend metadata walk. Normal HIR validation
    // rejects a map where the language contract requires String, before backend lowering.
    let map_value = expression(
        1,
        HirExpressionKind::MapLiteral(vec![HirMapEntry {
            key: string_expression(2, "key", types.string, RegionId(0)),
            value: int_expression(3, 1, types.int, RegionId(0)),
        }]),
        types.map_string_int,
        RegionId(0),
        ValueKind::RValue,
    );
    let message = optional_message(4, option_string, map_value, 1, ValueKind::RValue);
    let source = function_with_assertion(
        vec![HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::AssertFailure {
                message,
                message_evaluation: HirAssertionMessageEvaluation::Runtime,
            },
        }],
        &mut string_table,
        &type_environment,
        &types,
        "map_assertion",
        &[],
    );

    assert!(source.contains("function __moth_map_new("));
    assert!(source.contains("__moth_map_new("));
}

#[test]
fn reactive_assertion_message_emits_failure_snapshot_helpers() {
    let mut string_table = StringTable::new();
    let (mut type_environment, types) = build_type_environment();
    let option_string = type_environment.intern_option(types.string);
    let source_local = LocalId(0);
    let region = RegionId(0);
    let message_value = expression(
        2,
        HirExpressionKind::Load(HirPlace::Local(source_local)),
        types.string,
        region,
        ValueKind::RValue,
    );
    let message = optional_message(3, option_string, message_value, 1, ValueKind::RValue);
    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(source_local.0, types.string, region)],
        statements: vec![statement(
            4,
            HirStatementKind::Assign {
                target: HirPlace::Local(source_local),
                value: string_expression(5, "reactive message", types.string, region),
            },
            1,
        )],
        terminator: HirTerminator::AssertFailure {
            message,
            message_evaluation: HirAssertionMessageEvaluation::Runtime,
        },
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };
    let mut module = build_module(
        &mut string_table,
        "reactive_assertion",
        vec![block],
        function,
        &[(source_local, "message")],
    );
    let source_path = InternedPath::from_single_str("message", &mut string_table);
    module.side_table.bind_reactive_source(HirReactiveSource {
        id: ReactiveSourceId(0),
        local_id: source_local,
        path: source_path,
        kind: HirReactiveSourceKind::Declaration,
        type_id: types.string,
        location: test_source_location(1),
    });
    module
        .side_table
        .bind_reactive_template(HirReactiveTemplate {
            id: ReactiveTemplateId(0),
            value_id: HirValueId(2),
            dependencies: vec![HirReactiveTemplateDependency {
                source: ReactiveSourceId(0),
                type_id: types.string,
                location: test_source_location(1),
            }],
            template_value_parameters: vec![],
            template_backed: false,
            location: test_source_location(2),
        });

    let source = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("reactive assertion lowering should succeed")
    .source;

    assert!(source.contains("function __moth_template_string("));
    assert!(source.contains("function __moth_template_snapshot("));
    assert!(source.contains("__moth_template_snapshot(__moth_template_string("));
    assert!(source.contains("let __assert_message_0"));
}
