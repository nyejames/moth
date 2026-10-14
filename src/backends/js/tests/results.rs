//! Native result-channel emission tests and JavaScript ABI packing contracts.

use super::support::*;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{
    BlockId, FunctionId, HirNodeId, HirValueId, LocalId, RegionId,
};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::{HirLocalDestination, HirStatementKind};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};

fn load(expressions: &mut HirExpressionStore, local: u32, ty: TypeId) -> HirValueId {
    expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(local))),
        ty,
        RegionId(0),
        ValueKind::RValue,
        expressions,
    )
}

/// Publishes only native HIR slots; envelopes are introduced by JS lowering, not fixtures.
fn result_module(
    mut expressions: HirExpressionStore,
    blocks: Vec<HirBlock>,
    functions: Vec<HirFunction>,
    names: &[&str],
    start: FunctionId,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> HirModule {
    let mut module = HirModule::new();
    expressions.freeze();
    module.expressions = expressions;
    module.blocks = blocks;
    module.functions = functions;
    module.start_function = Some(start);
    module.regions = vec![HirRegion::lexical(RegionId(0), None)];
    for (function, name) in module.functions.iter().zip(names) {
        module.side_table.bind_function_name(
            function.id,
            path_fork
                .try_intern_portable_path(name, string_table)
                .expect("test path fits"),
        );
        module
            .function_origins
            .insert(function.id, HirFunctionOrigin::Normal);
    }
    module
}

fn assert_runtime_output(source: &str, invocation: &str, expected: &str) {
    let script = format!("{source}\nconsole.log(JSON.stringify({invocation}));");
    let runtime = std::process::Command::new("node")
        .args(["--eval", &script])
        .output()
        .expect("Node.js is required for result-channel runtime tests");
    assert!(
        runtime.status.success(),
        "result-channel runtime failed: {}",
        String::from_utf8_lossy(&runtime.stderr)
    );
    assert_eq!(
        String::from_utf8(runtime.stdout)
            .expect("Node output should be UTF-8")
            .trim(),
        expected
    );
}

/// Each nested call executes once, defines its selected edge and propagates native slots.
#[test]
fn nested_fallible_calls_emit_native_invoke_edges() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let mut blocks = vec![HirBlock {
        id: BlockId(0),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(append_values(
            &[string_expression(
                "inner",
                types.string,
                region,
                &mut expressions,
            )],
            &mut expressions,
        )),
    }];
    let mut functions = vec![HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        returns: vec![types.string].into_boxed_slice(),
        error_return: Some(types.string),
    }];
    for caller in 1..=2 {
        let entry = 1 + (caller - 1) * 3;
        let success_local = (caller - 1) * 2;
        let error_local = success_local + 1;
        blocks.extend([
            HirBlock {
                id: BlockId(entry),
                region,
                locals: vec![
                    local(success_local, types.string, region),
                    local(error_local, types.string, region),
                ],
                statements: vec![],
                terminator: HirTerminator::Invoke {
                    id: HirNodeId(caller),
                    span: None,
                    operation: HirFallibleOperation::Call {
                        target: CallTarget::Local(FunctionId(caller - 1)),
                        args: append_values(&[], &mut expressions),
                    },
                    success: BlockId(entry + 1),
                    success_results: vec![LocalId(success_local)],
                    failure: BlockId(entry + 2),
                    error_result: LocalId(error_local),
                },
            },
            HirBlock {
                id: BlockId(entry + 1),
                region,
                locals: vec![],
                statements: vec![],
                terminator: HirTerminator::Return(append_values(
                    &[load(&mut expressions, success_local, types.string)],
                    &mut expressions,
                )),
            },
            HirBlock {
                id: BlockId(entry + 2),
                region,
                locals: vec![],
                statements: vec![],
                terminator: HirTerminator::ReturnError(load(
                    &mut expressions,
                    error_local,
                    types.string,
                )),
            },
        ]);
        functions.push(HirFunction {
            id: FunctionId(caller),
            entry: BlockId(entry),
            params: vec![],
            returns: vec![types.string].into_boxed_slice(),
            error_return: Some(types.string),
        });
    }
    let module = result_module(
        expressions,
        blocks,
        functions,
        &["inner", "middle", "outer"],
        FunctionId(2),
        &mut path_fork,
        &mut string_table,
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
    for (name, id, callee) in [("middle", 1, "inner"), ("outer", 2, "middle")] {
        let body = helper_source(&output.source, &expected_dev_function_name(name, id));
        assert_eq!(
            body.matches(&format!("{}(", expected_dev_function_name(callee, id - 1)))
                .count(),
            1,
            "Invoke evaluates its operation once"
        );
        assert!(
            body.contains(".tag === \"ok\""),
            "Invoke owns the ABI tag check"
        );
        assert!(!body.contains("__moth_result_propagate"));
    }
    assert_runtime_output(
        &output.source,
        &format!("{}()", expected_dev_function_name("outer", 2)),
        r#"{"tag":"ok","value":"inner"}"#,
    );
}

/// Error-only signatures have no synthetic unit success slot and pack errors at the ABI.
#[test]
fn explicit_error_return_terminator_emits_err_envelope() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::ReturnError(string_expression(
            "boom",
            types.string,
            region,
            &mut expressions,
        )),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        returns: vec![].into_boxed_slice(),
        error_return: Some(types.string),
    };
    let module = result_module(
        expressions,
        vec![block],
        vec![function],
        &["fail"],
        FunctionId(0),
        &mut path_fork,
        &mut string_table,
    );
    let output = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("JS lowering should emit ReturnError");
    assert!(
        output
            .source
            .contains("return { tag: \"err\", value: \"boom\" };")
    );
    assert_runtime_output(
        &output.source,
        &format!("{}()", expected_dev_function_name("fail", 0)),
        r#"{"tag":"err","value":"boom"}"#,
    );
}

/// Multiple success slots survive Invoke transport and are packed only in the JS return ABI.
#[test]
fn multiple_success_return_slots_pack_at_js_abi() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(append_values(
            &[
                string_expression("ok", types.string, region, &mut expressions),
                i32_expression(42, types.i32, region, &mut expressions),
            ],
            &mut expressions,
        )),
    };
    let caller_entry = HirBlock {
        id: BlockId(1),
        region,
        locals: vec![
            local(0, types.string, region),
            local(1, types.i32, region),
            local(2, types.string, region),
        ],
        statements: vec![],
        terminator: HirTerminator::Invoke {
            id: HirNodeId(0),
            span: None,
            operation: HirFallibleOperation::Call {
                target: CallTarget::Local(FunctionId(0)),
                args: append_values(&[], &mut expressions),
            },
            success: BlockId(2),
            success_results: vec![LocalId(0), LocalId(1)],
            failure: BlockId(3),
            error_result: LocalId(2),
        },
    };
    let caller_success = HirBlock {
        id: BlockId(2),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(append_values(
            &[
                load(&mut expressions, 0, types.string),
                load(&mut expressions, 1, types.i32),
            ],
            &mut expressions,
        )),
    };
    let caller_error = HirBlock {
        id: BlockId(3),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::ReturnError(load(&mut expressions, 2, types.string)),
    };
    let functions = vec![
        HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![],
            returns: vec![types.string, types.i32].into_boxed_slice(),
            error_return: Some(types.string),
        },
        HirFunction {
            id: FunctionId(1),
            entry: BlockId(1),
            params: vec![],
            returns: vec![types.string, types.i32].into_boxed_slice(),
            error_return: Some(types.string),
        },
    ];
    let module = result_module(
        expressions,
        vec![block, caller_entry, caller_success, caller_error],
        functions,
        &["pair", "forward_pair"],
        FunctionId(1),
        &mut path_fork,
        &mut string_table,
    );
    let output = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("JS lowering should emit ordered Return slots");
    assert!(
        output
            .source
            .contains("return { tag: \"ok\", value: [\"ok\", 42] };"),
        "ABI packing must preserve success-slot order"
    );
    assert_runtime_output(
        &output.source,
        &format!("{}()", expected_dev_function_name("forward_pair", 1)),
        r#"{"tag":"ok","value":["ok",42]}"#,
    );
}

/// Zero-success Invoke still routes success/error edges without a carrier or unit local.
#[test]
fn zero_success_invoke_emits_success_error_edges() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region,
            locals: vec![local(0, types.boolean, region)],
            statements: vec![],
            terminator: HirTerminator::If {
                condition: load(&mut expressions, 0, types.boolean),
                then_block: BlockId(1),
                else_block: BlockId(2),
            },
        },
        HirBlock {
            id: BlockId(1),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(append_values(&[], &mut expressions)),
        },
        HirBlock {
            id: BlockId(2),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::ReturnError(string_expression(
                "boom",
                types.string,
                region,
                &mut expressions,
            )),
        },
        HirBlock {
            id: BlockId(3),
            region,
            locals: vec![
                local(1, types.boolean, region),
                local(2, types.string, region),
            ],
            statements: vec![],
            terminator: HirTerminator::Invoke {
                id: HirNodeId(0),
                span: None,
                operation: HirFallibleOperation::Call {
                    target: CallTarget::Local(FunctionId(0)),
                    args: append_values(
                        &[load(&mut expressions, 1, types.boolean)],
                        &mut expressions,
                    ),
                },
                success: BlockId(4),
                success_results: vec![],
                failure: BlockId(5),
                error_result: LocalId(2),
            },
        },
        HirBlock {
            id: BlockId(4),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(append_values(
                &[bool_expression(
                    true,
                    types.boolean,
                    region,
                    &mut expressions,
                )],
                &mut expressions,
            )),
        },
        HirBlock {
            id: BlockId(5),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(append_values(
                &[bool_expression(
                    false,
                    types.boolean,
                    region,
                    &mut expressions,
                )],
                &mut expressions,
            )),
        },
    ];
    let functions = vec![
        HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![LocalId(0)],
            returns: vec![].into_boxed_slice(),
            error_return: Some(types.string),
        },
        HirFunction {
            id: FunctionId(1),
            entry: BlockId(3),
            params: vec![LocalId(1)],
            returns: vec![types.boolean].into_boxed_slice(),
            error_return: None,
        },
    ];
    let module = result_module(
        expressions,
        blocks,
        functions,
        &["validate", "branch_on_result"],
        FunctionId(1),
        &mut path_fork,
        &mut string_table,
    );
    let output = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("JS lowering should emit zero-success Invoke");
    let body = helper_source(
        &output.source,
        &expected_dev_function_name("branch_on_result", 1),
    );
    assert!(body.contains(".tag === \"ok\""));
    let name = expected_dev_function_name("branch_on_result", 1);
    assert_runtime_output(
        &output.source,
        &format!("[{name}(true), {name}(false)]"),
        "[true,false]",
    );
}

/// The success destination is a fresh caller binding, even when its payload aliases a parameter.
#[test]
fn fallible_alias_return_defines_fresh_success_binding() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region,
            locals: vec![local(0, types.string, region)],
            statements: vec![],
            terminator: HirTerminator::Return(append_values(
                &[load(&mut expressions, 0, types.string)],
                &mut expressions,
            )),
        },
        HirBlock {
            id: BlockId(1),
            region,
            locals: vec![
                local(1, types.string, region),
                local(2, types.string, region),
                local(3, types.string, region),
            ],
            statements: vec![],
            terminator: HirTerminator::Invoke {
                id: HirNodeId(0),
                span: None,
                operation: HirFallibleOperation::Call {
                    target: CallTarget::Local(FunctionId(0)),
                    args: append_values(
                        &[load(&mut expressions, 1, types.string)],
                        &mut expressions,
                    ),
                },
                success: BlockId(2),
                success_results: vec![LocalId(2)],
                failure: BlockId(3),
                error_result: LocalId(3),
            },
        },
        HirBlock {
            id: BlockId(2),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(append_values(
                &[load(&mut expressions, 2, types.string)],
                &mut expressions,
            )),
        },
        HirBlock {
            id: BlockId(3),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::ReturnError(load(&mut expressions, 3, types.string)),
        },
    ];
    let functions = vec![
        HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![LocalId(0)],
            returns: vec![types.string].into_boxed_slice(),
            error_return: Some(types.string),
        },
        HirFunction {
            id: FunctionId(1),
            entry: BlockId(1),
            params: vec![LocalId(1)],
            returns: vec![types.string].into_boxed_slice(),
            error_return: Some(types.string),
        },
    ];
    let mut module = result_module(
        expressions,
        blocks,
        functions,
        &["aliasing_fallible", "main"],
        FunctionId(1),
        &mut path_fork,
        &mut string_table,
    );
    module.side_table.bind_local_name(
        LocalId(2),
        path_fork
            .try_intern_portable_path("success", &mut string_table)
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
    .expect("JS lowering should emit alias-return Invoke");
    let body = helper_source(&output.source, &expected_dev_function_name("main", 1));
    let result_name = expected_dev_local_name("success", 2);
    assert!(
        body.contains(&format!("{result_name} = __moth_binding(__invoke_result_")),
        "success edges create a separate caller binding"
    );
    assert!(!body.contains(&format!("__moth_assign_borrow({result_name},")));
    let callee_body = helper_source(
        &output.source,
        &expected_dev_function_name("aliasing_fallible", 0),
    );
    assert!(callee_body.contains("return { tag: \"ok\", value: __moth_read("));
    assert!(!callee_body.contains("__moth_clone_value(__moth_read("));
    assert_runtime_output(
        &output.source,
        &format!("{}(\"shared\")", expected_dev_function_name("main", 1)),
        r#"{"tag":"ok","value":"shared"}"#,
    );
}

/// Infallible calls bind every ordered result without introducing tuple-typed HIR values.
#[test]
fn infallible_call_defines_multiple_result_slots() {
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(append_values(
                &[
                    string_expression("first", types.string, region, &mut expressions),
                    i32_expression(2, types.i32, region, &mut expressions),
                ],
                &mut expressions,
            )),
        },
        HirBlock {
            id: BlockId(1),
            region,
            locals: vec![local(0, types.string, region), local(1, types.i32, region)],
            statements: vec![statement(
                0,
                HirStatementKind::Call {
                    target: CallTarget::Local(FunctionId(0)),
                    args: append_values(&[], &mut expressions),
                    results: vec![
                        HirLocalDestination::Define(LocalId(0)),
                        HirLocalDestination::Define(LocalId(1)),
                    ],
                },
            )],
            terminator: HirTerminator::Return(append_values(
                &[
                    load(&mut expressions, 0, types.string),
                    load(&mut expressions, 1, types.i32),
                ],
                &mut expressions,
            )),
        },
    ];
    let functions = vec![
        HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![],
            returns: vec![types.string, types.i32].into_boxed_slice(),
            error_return: None,
        },
        HirFunction {
            id: FunctionId(1),
            entry: BlockId(1),
            params: vec![],
            returns: vec![types.string, types.i32].into_boxed_slice(),
            error_return: None,
        },
    ];
    let module = result_module(
        expressions,
        blocks,
        functions,
        &["pair", "main"],
        FunctionId(1),
        &mut path_fork,
        &mut string_table,
    );
    let output = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("JS lowering should bind all Call results");
    let body = helper_source(&output.source, &expected_dev_function_name("main", 1));
    assert_eq!(
        body.matches(&format!("{}(", expected_dev_function_name("pair", 0)))
            .count(),
        1
    );
    assert_runtime_output(
        &output.source,
        &format!("{}()", expected_dev_function_name("main", 1)),
        r#"["first",2]"#,
    );
}
