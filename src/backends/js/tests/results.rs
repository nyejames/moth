//! Fallible propagation emission tests for JavaScript output.

use super::support::*;
use crate::compiler_frontend::datatypes::ids::{BuiltinTypeConstructor, TypeConstructor};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;

// Fallible emission contract tests beyond helper-level [result]
// ----------------------------------------------------------

/// Verifies that nested fallible functions branch on the explicit HIR carrier rather than
/// lowering through the obsolete expression-level propagation helper. [result]
#[test]
fn nested_fallible_calls_emit_explicit_carrier_branches() {
    let mut string_table = StringTable::new();
    let (mut type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let result_type = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier),
        Box::new([types.string, types.string]),
    );

    let result_load = |id, local| {
        expression(
            id,
            HirExpressionKind::Load(HirPlace::Local(local)),
            result_type,
            region,
            ValueKind::RValue,
        )
    };
    let ok_payload = |id, inner_id, local| {
        expression(
            id,
            HirExpressionKind::FallibleUnwrapSuccess {
                result: Box::new(result_load(inner_id, local)),
            },
            types.string,
            region,
            ValueKind::RValue,
        )
    };
    let err_payload = |id, inner_id, local| {
        expression(
            id,
            HirExpressionKind::FallibleUnwrapError {
                result: Box::new(result_load(inner_id, local)),
            },
            types.string,
            region,
            ValueKind::RValue,
        )
    };

    // Function C (id 0): innermost fallible function succeeds directly.
    let c_block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::ReturnSuccess(string_expression(
            1,
            "inner",
            types.string,
            region,
        )),
    };
    let c_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: result_type,
        return_aliases: vec![],
    };

    // Function B (id 1): calls C and propagates through explicit carrier edges.
    let b_call = statement(
        3,
        HirStatementKind::Call {
            target: CallTarget::UserFunction(FunctionId(0)),
            args: vec![],
            result: Some(LocalId(0)),
        },
        1,
    );
    let b_entry = HirBlock {
        id: BlockId(1),
        region,
        locals: vec![local(0, result_type, region)],
        statements: vec![b_call],
        terminator: HirTerminator::FallibleBranch {
            result: result_load(4, LocalId(0)),
            success_block: BlockId(2),
            error_block: BlockId(3),
        },
    };
    let b_success = HirBlock {
        id: BlockId(2),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::ReturnSuccess(ok_payload(5, 6, LocalId(0))),
    };
    let b_error = HirBlock {
        id: BlockId(3),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::ReturnError(err_payload(7, 8, LocalId(0))),
    };
    let b_function = HirFunction {
        id: FunctionId(1),
        entry: BlockId(1),
        params: vec![],
        return_type: result_type,
        return_aliases: vec![],
    };

    // Function A (id 2): calls B and uses the same explicit branch shape.
    let a_call = statement(
        7,
        HirStatementKind::Call {
            target: CallTarget::UserFunction(FunctionId(1)),
            args: vec![],
            result: Some(LocalId(1)),
        },
        1,
    );
    let a_entry = HirBlock {
        id: BlockId(4),
        region,
        locals: vec![local(1, result_type, region)],
        statements: vec![a_call],
        terminator: HirTerminator::FallibleBranch {
            result: result_load(9, LocalId(1)),
            success_block: BlockId(5),
            error_block: BlockId(6),
        },
    };
    let a_success = HirBlock {
        id: BlockId(5),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::ReturnSuccess(ok_payload(10, 11, LocalId(1))),
    };
    let a_error = HirBlock {
        id: BlockId(6),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::ReturnError(err_payload(12, 13, LocalId(1))),
    };
    let a_function = HirFunction {
        id: FunctionId(2),
        entry: BlockId(4),
        params: vec![],
        return_type: result_type,
        return_aliases: vec![],
    };

    let mut module = HirModule::new();
    module.blocks = vec![
        c_block, b_entry, b_success, b_error, a_entry, a_success, a_error,
    ];
    module.functions = vec![c_function, b_function, a_function];
    module.start_function = FunctionId(2);
    module.regions = vec![HirRegion::lexical(RegionId(0), None)];

    module.side_table.bind_function_name(
        FunctionId(0),
        InternedPath::from_single_str("inner", &mut string_table),
    );
    module.side_table.bind_function_name(
        FunctionId(1),
        InternedPath::from_single_str("middle", &mut string_table),
    );
    module.side_table.bind_function_name(
        FunctionId(2),
        InternedPath::from_single_str("outer", &mut string_table),
    );
    module.side_table.bind_local_name(
        LocalId(0),
        InternedPath::from_single_str("middle_result", &mut string_table),
    );
    module.side_table.bind_local_name(
        LocalId(1),
        InternedPath::from_single_str("outer_result", &mut string_table),
    );

    module
        .function_origins
        .insert(FunctionId(0), HirFunctionOrigin::Normal);
    module
        .function_origins
        .insert(FunctionId(1), HirFunctionOrigin::Normal);
    module
        .function_origins
        .insert(FunctionId(2), HirFunctionOrigin::Normal);

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    let try_count = output.source.matches("try {").count();
    assert!(
        try_count >= 3,
        "expected at least 3 try/catch wrappers for fallible-returning functions, found {try_count}"
    );

    let middle_result = expected_dev_local_name("middle_result", 0);
    let outer_result = expected_dev_local_name("outer_result", 1);

    assert!(
        output
            .source
            .contains(&format!("((__moth_read({middle_result})).tag === \"ok\")")),
        "middle function should branch on the explicit fallible carrier local"
    );
    assert!(
        output
            .source
            .contains(&format!("((__moth_read({outer_result})).tag === \"ok\")")),
        "outer function should branch on the explicit fallible carrier local"
    );
    assert!(
        !output
            .source
            .contains("return { tag: \"ok\", value: __moth_result_propagate("),
        "nested fallible functions should not return through the old expression-level propagation helper"
    );
}

/// Verifies that explicit HIR error-return edges still emit the current JS fallible-carrier ABI.
/// [result]
#[test]
fn explicit_error_return_terminator_emits_err_carrier() {
    let mut string_table = StringTable::new();
    let (mut type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let result_type = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier),
        Box::new([types.unit, types.string]),
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::ReturnError(string_expression(1, "boom", types.string, region)),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: result_type,
        return_aliases: vec![],
    };

    let mut module = HirModule::new();
    module.blocks = vec![block];
    module.functions = vec![function];
    module.start_function = FunctionId(0);
    module.regions = vec![HirRegion::lexical(RegionId(0), None)];
    module.side_table.bind_function_name(
        FunctionId(0),
        InternedPath::from_single_str("fail", &mut string_table),
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
    .expect("JS lowering should emit ReturnError");

    assert!(
        output
            .source
            .contains("return { tag: \"err\", value: \"boom\" };"),
        "ReturnError should lower to the current JS error carrier"
    );
}

/// Verifies that explicit HIR success-return edges still emit the current JS success carrier.
/// [result]
#[test]
fn explicit_success_return_terminator_emits_ok_carrier() {
    let mut string_table = StringTable::new();
    let (mut type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let result_type = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier),
        Box::new([types.string, types.string]),
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::ReturnSuccess(string_expression(1, "ok", types.string, region)),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: result_type,
        return_aliases: vec![],
    };

    let mut module = HirModule::new();
    module.blocks = vec![block];
    module.functions = vec![function];
    module.start_function = FunctionId(0);
    module.regions = vec![HirRegion::lexical(RegionId(0), None)];
    module.side_table.bind_function_name(
        FunctionId(0),
        InternedPath::from_single_str("succeed", &mut string_table),
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
    .expect("JS lowering should emit ReturnSuccess");

    assert!(
        output
            .source
            .contains("return { tag: \"ok\", value: \"ok\" };"),
        "ReturnSuccess should lower to the current JS success carrier"
    );
}

/// Verifies that the explicit HIR fallible branch terminator owns the JS tag check instead of
/// requiring a separate boolean fallible-carrier expression node. [result]
#[test]
fn fallible_branch_terminator_emits_success_error_tag_branch() {
    let mut string_table = StringTable::new();
    let (mut type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let result_type = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier),
        Box::new([types.unit, types.string]),
    );

    let entry_block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, result_type, region)],
        statements: vec![],
        terminator: HirTerminator::FallibleBranch {
            result: expression(
                1,
                HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                result_type,
                region,
                ValueKind::RValue,
            ),
            success_block: BlockId(1),
            error_block: BlockId(2),
        },
    };
    let success_block = HirBlock {
        id: BlockId(1),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(unit_expression(2, types.unit, region)),
    };
    let error_block = HirBlock {
        id: BlockId(2),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(unit_expression(3, types.unit, region)),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
        return_aliases: vec![],
    };

    let mut module = HirModule::new();
    module.blocks = vec![entry_block, success_block, error_block];
    module.functions = vec![function];
    module.start_function = FunctionId(0);
    module.regions = vec![HirRegion::lexical(RegionId(0), None)];
    module.side_table.bind_function_name(
        FunctionId(0),
        InternedPath::from_single_str("branch_on_result", &mut string_table),
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
    .expect("JS lowering should emit FallibleBranch");

    assert!(
        output.source.contains(".tag === \"ok\""),
        "FallibleBranch should lower to a backend-owned success/error tag check"
    );
    assert!(
        !output.source.contains("result_is_ok"),
        "FallibleBranch should not require a separate carrier-test expression helper"
    );
}

/// Verifies that alias metadata for a fallible function applies to the success payload, not the
/// fallible carrier local that receives the call. [result] [alias]
#[test]
fn fallible_alias_return_call_assigns_result_carrier_as_fresh_value() {
    let mut string_table = StringTable::new();
    let (mut type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let result_type = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier),
        Box::new([types.string, types.string]),
    );

    let callee_block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, types.string, region)],
        statements: vec![],
        terminator: HirTerminator::ReturnSuccess(expression(
            1,
            HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
            types.string,
            region,
            ValueKind::RValue,
        )),
    };
    let callee = HirFunction {
        id: FunctionId(1),
        entry: BlockId(0),
        params: vec![LocalId(0)],
        return_type: result_type,
        return_aliases: vec![Some(vec![0])],
    };

    let call_aliasing_fallible = statement(
        2,
        HirStatementKind::Call {
            target: CallTarget::UserFunction(FunctionId(1)),
            args: vec![expression(
                3,
                HirExpressionKind::Load(HirPlace::Local(LocalId(1))),
                types.string,
                region,
                ValueKind::Place,
            )],
            result: Some(LocalId(2)),
        },
        2,
    );
    let caller_block = HirBlock {
        id: BlockId(1),
        region,
        locals: vec![
            local(1, types.string, region),
            local(2, result_type, region),
        ],
        statements: vec![call_aliasing_fallible],
        terminator: HirTerminator::Return(unit_expression(4, types.unit, region)),
    };
    let caller = HirFunction {
        id: FunctionId(0),
        entry: BlockId(1),
        params: vec![],
        return_type: types.unit,
        return_aliases: vec![],
    };

    let mut module = HirModule::new();
    module.blocks = vec![callee_block, caller_block];
    module.functions = vec![caller, callee];
    module.start_function = FunctionId(0);
    module.regions = vec![HirRegion::lexical(region, None)];
    module.side_table.bind_function_name(
        FunctionId(0),
        InternedPath::from_single_str("main", &mut string_table),
    );
    module.side_table.bind_function_name(
        FunctionId(1),
        InternedPath::from_single_str("aliasing_fallible", &mut string_table),
    );
    module.side_table.bind_local_name(
        LocalId(1),
        InternedPath::from_single_str("source", &mut string_table),
    );
    module.side_table.bind_local_name(
        LocalId(2),
        InternedPath::from_single_str("result_carrier", &mut string_table),
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
    .expect("JS lowering should emit fallible alias-return call");

    let result_name = expected_dev_local_name("result_carrier", 2);
    let callee_name = expected_dev_function_name("aliasing_fallible", 1);

    assert!(
        output.source.contains(&format!(
            "__moth_assign_value({result_name}, {callee_name}("
        )),
        "fallible call result carriers must be stored as fresh values"
    );
    assert!(
        !output.source.contains(&format!(
            "__moth_assign_borrow({result_name}, {callee_name}("
        )),
        "fallible call result carriers must not inherit the success payload alias mode"
    );
}
