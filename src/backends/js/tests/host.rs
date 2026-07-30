//! Host-function and start-invocation JavaScript emission tests.

use super::support::*;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use std::sync::Arc;

// Host function and start-invocation tests [host] [start]
// ---------------------------------------------------------------------------

/// Verifies that host io.line([: [...]]) reads the binding value before logging. [host]
#[test]
fn host_io_reads_the_underlying_value_before_logging() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let io_id = crate::compiler_frontend::external_packages::ExternalFunctionId::IoLine;

    let assign_message = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: string_expression(1, "hello", types.string, RegionId(0)),
        },
        1,
    );

    let call_statement = statement(
        2,
        HirStatementKind::Call {
            target: CallTarget::External(io_id),
            args: vec![expression(
                2,
                HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                types.string,
                RegionId(0),
                ValueKind::RValue,
            )],
            result: None,
        },
        2,
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.string, RegionId(0))],
        statements: vec![assign_message, call_statement],
        terminator: HirTerminator::Return(unit_expression(3, types.unit, RegionId(0))),
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
        "entry_start",
        vec![block],
        function,
        &[(LocalId(0), "message")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        JsLoweringConfig {
            pretty: true,
            emit_locations: false,
            auto_invoke_start: true,
            function_emission_policy: JsFunctionEmissionPolicy::AllFunctions,
            external_package_registry: Arc::new(ExternalPackageRegistry::new()),
            external_module_export_glue_enabled: false,
            source_function_names: Arc::new(Default::default()),
            module_private_function_names: Arc::new(Default::default()),
            generated_function_names: Arc::new(Default::default()),
        },
        &type_environment,
    )
    .expect("JS lowering should succeed");
    let message_name = expected_dev_local_name("message", 0);

    let assign_index = output
        .source
        .find(&format!("__moth_assign_value({message_name}, \"hello\");"))
        .expect("expected local assignment to store the string value");
    let log_index = output
        .source
        .find(&format!("__moth_io_line(__moth_read({message_name}));"))
        .expect("expected host io call to read from the local binding");

    assert!(
        assign_index < log_index,
        "host logging should occur after assigning the local value"
    );
}

/// Verifies that auto_invoke_start emits a call to the start function. [start]
#[test]
fn auto_invokes_start_function_when_enabled() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(unit_expression(0, types.unit, RegionId(0))),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
        return_aliases: vec![],
    };

    let module = build_module(&mut string_table, "start_main", vec![block], function, &[]);

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        JsLoweringConfig {
            pretty: true,
            emit_locations: false,
            auto_invoke_start: true,
            function_emission_policy: JsFunctionEmissionPolicy::AllFunctions,
            external_package_registry: Arc::new(ExternalPackageRegistry::new()),
            external_module_export_glue_enabled: false,
            source_function_names: Arc::new(Default::default()),
            module_private_function_names: Arc::new(Default::default()),
            generated_function_names: Arc::new(Default::default()),
        },
        &type_environment,
    )
    .expect("JS lowering should succeed");
    let start_name = expected_dev_function_name("start_main", 0);

    assert!(output.source.contains(&format!("{start_name}();")));
}

// ---------------------------------------------------------------------------
