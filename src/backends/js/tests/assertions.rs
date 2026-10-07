//! JavaScript lowering tests for assertion failure messages.
//!
//! WHAT: pins one-time failure-edge evaluation and optional-message selection for both JS CFG
//!       lowering strategies.
//! WHY: assertion messages are ordinary HIR values, so the backend must lower the value exactly
//!      once at the failure terminator without eagerly evaluating successful assertions.

use super::support::*;
use crate::backends::js::test_symbol_helpers::expected_dev_function_name;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{
    HirExpressionKind, HirMapEntry, HirVariantCarrier, HirVariantField, ValueKind,
};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::terminators::{HirAssertionMessageEvaluation, HirTerminator};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

fn optional_message(
    type_id: crate::compiler_frontend::datatypes::ids::TypeId,
    inner: crate::compiler_frontend::hir::ids::HirValueId,
    variant_index: usize,
    value_kind: ValueKind,
    expressions: &mut HirExpressionStore,
) -> crate::compiler_frontend::hir::ids::HirValueId {
    let fields = if variant_index == 0 {
        append_variant_fields(&[], expressions)
    } else {
        append_variant_fields(
            &[HirVariantField {
                name: None,
                value: inner,
            }],
            expressions,
        )
    };
    expression(
        HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Option,
            variant_index,
            fields,
        },
        type_id,
        RegionId(0),
        value_kind,
        expressions,
    )
}

fn function_with_assertion(
    path_fork: &mut PathInternerFork,
    blocks: Vec<HirBlock>,
    string_table: &mut StringTable,
    type_environment: &crate::compiler_frontend::datatypes::environment::TypeEnvironment,
    function_name: &str,
    local_names: &[(LocalId, &str)],
    expressions: HirExpressionStore,
) -> String {
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: type_environment.builtins().none,
    };
    let module = build_module(
        expressions,
        path_fork,
        string_table,
        function_name,
        blocks,
        function,
        local_names,
    );

    lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        string_table,
        default_config(),
        type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("JS assertion lowering should succeed")
    .source
}

#[test]
fn structured_assertion_message_is_lowered_once_and_selected() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (mut type_environment, types) = build_type_environment();
    let option_string = type_environment.intern_option(types.string);
    let casted_message = expression(
        HirExpressionKind::Cast {
            source: expression(
                HirExpressionKind::Load(HirPlace::local(LocalId(0))),
                types.int,
                RegionId(0),
                ValueKind::RValue,
                &mut expressions,
            ),
            policy: BuiltinCastPolicyId::NumericToString(NumericScalar::Int),
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );
    let message = optional_message(
        option_string,
        casted_message,
        1,
        ValueKind::RValue,
        &mut expressions,
    );
    let source = function_with_assertion(
        &mut path_fork,
        vec![HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![local(0, types.int, RegionId(0))],
            statements: vec![statement(
                4,
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(LocalId(0)),
                    value: int_expression(42, types.int, RegionId(0), &mut expressions),
                },
            )],
            terminator: HirTerminator::AssertFailure {
                message,
                message_evaluation: HirAssertionMessageEvaluation::Runtime,
            },
        }],
        &mut string_table,
        &type_environment,
        "structured_assertion",
        &[(LocalId(0), "value")],
        expressions,
    );

    assert_eq!(source.matches("let __assert_message_").count(), 1);
    assert_eq!(
        source
            .matches("throw __moth_assertion_error((__assert_message_")
            .count(),
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
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (mut type_environment, types) = build_type_environment();
    let option_string = type_environment.intern_option(types.string);
    let casted_message = expression(
        HirExpressionKind::Cast {
            source: expression(
                HirExpressionKind::Load(HirPlace::local(LocalId(0))),
                types.boolean,
                RegionId(0),
                ValueKind::RValue,
                &mut expressions,
            ),
            policy: BuiltinCastPolicyId::BoolToString,
        },
        types.string,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );
    let message = optional_message(
        option_string,
        casted_message,
        1,
        ValueKind::RValue,
        &mut expressions,
    );
    let source = function_with_assertion(
        &mut path_fork,
        vec![
            HirBlock {
                id: BlockId(0),
                region: RegionId(0),
                locals: vec![local(0, types.boolean, RegionId(0))],
                statements: vec![statement(
                    7,
                    HirStatementKind::Write {
                        target: HirWriteTarget::DefineLocal(LocalId(0)),
                        value: bool_expression(true, types.boolean, RegionId(0), &mut expressions),
                    },
                )],
                terminator: HirTerminator::If {
                    condition: bool_expression(true, types.boolean, RegionId(0), &mut expressions),
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
        "dispatcher_assertion",
        &[(LocalId(0), "flag")],
        expressions,
    );

    assert!(source.contains("switch (__bb"));
    assert_eq!(source.matches("let __assert_message_").count(), 1);
    assert_eq!(
        source
            .matches("throw __moth_assertion_error((__assert_message_")
            .count(),
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
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (mut type_environment, types) = build_type_environment();
    let option_string = type_environment.intern_option(types.string);
    let message = optional_message(
        option_string,
        string_expression(
            "unused message",
            types.string,
            RegionId(0),
            &mut expressions,
        ),
        0,
        ValueKind::Const,
        &mut expressions,
    );
    let source = function_with_assertion(
        &mut path_fork,
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
        "default_assertion",
        &[],
        expressions,
    );

    assert!(source.contains("throw __moth_assertion_error(\"assertion failed\");"));
    assert!(!source.contains("__assert_message_"));
    assert!(!source.contains("unused message"));
    let function_name = expected_dev_function_name("default_assertion", 0);
    let script = format!(
        "{source}\ntry {{ {function_name}(); }} catch (error) {{ console.log(JSON.stringify([error instanceof Error, error.message, Object.hasOwn(error, '__moth_assertion'), error.__moth_assertion, Object.getOwnPropertyDescriptor(error, '__moth_assertion').enumerable])); }}"
    );
    let output = std::process::Command::new("node")
        .args(["--eval", &script])
        .output()
        .expect("Node.js is required for assertion identity tests");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        r#"[true,"assertion failed",true,true,false]"#
    );
    assert!(!lower_minimal_module("no_assertions").contains("__moth_assertion"));
}

#[test]
fn assertion_message_map_metadata_emits_map_helpers() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (mut type_environment, types) = build_type_environment();
    let option_string = type_environment.intern_option(types.string);
    // This synthetic nested shape exercises the backend metadata walk. Normal HIR validation
    // rejects a map where the language contract requires String, before backend lowering.
    let map_value = expression(
        HirExpressionKind::MapLiteral(append_map_entries(
            &[HirMapEntry {
                key: string_expression("key", types.string, RegionId(0), &mut expressions),
                value: int_expression(1, types.int, RegionId(0), &mut expressions),
            }],
            &mut expressions,
        )),
        types.map_string_int,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );
    let message = optional_message(
        option_string,
        map_value,
        1,
        ValueKind::RValue,
        &mut expressions,
    );
    let source = function_with_assertion(
        &mut path_fork,
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
        "map_assertion",
        &[],
        expressions,
    );

    assert!(source.contains("function __moth_map_new("));
    assert!(source.contains("__moth_map_new("));
}
