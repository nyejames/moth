//! Never-call JavaScript artefact and runtime tests.
//!
//! WHAT: proves that a `HirTerminator::NeverCall` emits one ordinary call followed by the
//!       compiler-owned violated-contract throw, including when the callee is a returning
//!       external implementation.
//! WHY: a declared `-> !` contract has no success continuation. Control must not continue into
//!      code the frontend proved unreachable if a foreign implementation returns.

use super::support::*;
use crate::backends::js::NEVER_RETURNED_MESSAGE;
use crate::compiler_frontend::datatypes::CallableReturnShape;
use crate::compiler_frontend::external_packages::{
    ExternalFunctionDef, ExternalFunctionId, ExternalFunctionLowerings, ExternalJsLowering,
    ExternalPackageRegistry, external_never,
};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionReturnContract};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use std::sync::Arc;

fn never_call_module(
    expressions: HirExpressionStore,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
    callee_name: &str,
    start_terminator: HirTerminator,
    callee_terminator: HirTerminator,
    callee_contract: HirFunctionReturnContract,
) -> HirModule {
    let start_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: start_terminator,
    };
    let callee_block = HirBlock {
        id: BlockId(1),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: callee_terminator,
    };
    let start = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
            vec![],
            None,
        )),
    };
    let callee = HirFunction {
        id: FunctionId(1),
        entry: BlockId(1),
        params: vec![],
        return_contract: callee_contract,
    };

    let mut module = HirModule::new();
    let mut expressions = expressions;
    expressions.freeze();
    module.expressions = expressions;
    module.blocks = vec![start_block, callee_block];
    module.functions = vec![start, callee];
    module.start_function = Some(FunctionId(0));
    module.regions = vec![HirRegion::lexical(RegionId(0), None)];
    module.side_table.bind_function_name(
        FunctionId(0),
        path_fork
            .try_intern_portable_path("run", string_table)
            .expect("test path fits"),
    );
    module.side_table.bind_function_name(
        FunctionId(1),
        path_fork
            .try_intern_portable_path(callee_name, string_table)
            .expect("test path fits"),
    );
    module
}

fn throw_source(message: &str) -> String {
    format!(
        "throw new Error({});",
        crate::backends::js::escape_js_string(message)
    )
}

/// Verifies a local NeverCall emits the callee exactly once, then the defensive throw.
#[test]
fn never_call_emits_call_then_throw_exactly_once() {
    let expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, _) = build_type_environment();
    let module = never_call_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "halt",
        HirTerminator::NeverCall {
            target: CallTarget::Local(FunctionId(1)),
            args: HirValueRange::empty(),
        },
        HirTerminator::RuntimeFailure {
            message: "halted".to_owned(),
            cause: None,
        },
        HirFunctionReturnContract::Never,
    );

    let source = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("JS lowering should succeed")
    .source;

    let halt_name = expected_dev_function_name("halt", 1);
    let run_name = expected_dev_function_name("run", 0);
    let call = format!("{halt_name}();");
    let throw = throw_source(NEVER_RETURNED_MESSAGE);

    assert_eq!(
        source.matches(&call).count(),
        1,
        "the NeverCall must emit the callee exactly once:\n{source}"
    );
    assert_eq!(
        source.matches(&throw).count(),
        1,
        "the defensive throw must appear exactly once:\n{source}"
    );
    let call_index = source
        .find(&call)
        .expect("the NeverCall must emit the callee");
    let throw_index = source
        .find(&throw)
        .expect("the NeverCall must emit the defensive throw");
    assert!(
        call_index < throw_index,
        "the call must precede the defensive throw:\n{source}"
    );
    assert!(
        !source[call_index..throw_index].contains("return "),
        "a NeverCall must not return a value between the call and the throw:\n{source}"
    );
    assert!(
        source.contains(&format!("function {run_name}(")),
        "the start function should be emitted:\n{source}"
    );
}

/// Verifies a returning external Never implementation throws at the call site under Node.
#[test]
fn returning_external_never_function_throws_at_runtime() {
    let expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, _) = build_type_environment();
    let function_id = ExternalFunctionId::Synthetic(77);
    let mut registry = ExternalPackageRegistry::new();
    let package_id = registry
        .register_package(
            "@test/never",
            crate::builder_surface::PackageOrigin::ProjectLocal,
        )
        .expect("test package should register");
    registry
        .register_function_in_package(
            package_id,
            function_id,
            ExternalFunctionDef {
                name: "returns".to_owned(),
                parameters: Vec::new(),
                return_contract: external_never(),
                const_eval: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::InlineExpression(
                        "(() => { globalThis.__moth_never_calls = (globalThis.__moth_never_calls || 0) + 1; return 7; })()".to_owned(),
                    )),
                    wasm: None,
                },
            },
        )
        .expect("test Never export should register");

    let module = never_call_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "unused",
        HirTerminator::NeverCall {
            target: CallTarget::External(function_id),
            args: HirValueRange::empty(),
        },
        HirTerminator::RuntimeFailure {
            message: "unused".to_owned(),
            cause: None,
        },
        HirFunctionReturnContract::Never,
    );

    let mut config = default_config();
    config.external_package_registry = Arc::new(registry);
    let source = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        config,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("JS lowering should succeed")
    .source;

    let run_name = expected_dev_function_name("run", 0);
    let inline_call = "(() => { globalThis.__moth_never_calls = (globalThis.__moth_never_calls || 0) + 1; return 7; })();";
    assert_eq!(
        source.matches(inline_call).count(),
        1,
        "the returning external NeverCall must emit its target exactly once:\n{source}"
    );
    let throw = throw_source(NEVER_RETURNED_MESSAGE);
    assert!(
        source.contains(&throw),
        "the returning external NeverCall must emit the defensive throw:\n{source}"
    );

    let script = format!(
        "{source}\ntry {{ {run_name}(); console.log(\"returned\"); }} catch (error) {{ console.log(JSON.stringify([error instanceof Error, error.message, globalThis.__moth_never_calls])); }}"
    );
    let output = std::process::Command::new("node")
        .args(["--eval", &script])
        .output()
        .expect("Node.js is required for NeverCall runtime tests");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!(r#"[true,{:?},1]"#, NEVER_RETURNED_MESSAGE),
        "a returning external Never function must throw the compiler-owned contract error after exactly one call"
    );
}
