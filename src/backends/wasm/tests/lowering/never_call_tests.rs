//! Never-call Wasm LIR artefact tests.
//!
//! WHAT: proves that a terminating Never call lowers to a result-less `Call` followed by the
//!       existing LIR trap path, while an ordinary zero-success call keeps its native return.
//! WHY: a `-> !` callee has no success continuation, but an ordinary no-value function still
//!      completes through its native channel.

use super::test_support::{
    build_module, build_type_environment, default_borrow_facts, default_numeric_proofs, statement,
};
use crate::backends::wasm::backend::lower_hir_to_wasm_lir;
use crate::backends::wasm::lir::function::WasmLirFunctionOrigin;
use crate::backends::wasm::lir::instructions::{WasmCalleeRef, WasmLirStmt, WasmLirTerminator};
use crate::backends::wasm::request::WasmBackendRequest;
use crate::compiler_frontend::datatypes::CallableReturnShape;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expression_store::{HirExpressionStore, HirValueRange};
use crate::compiler_frontend::hir::functions::{
    HirFunction, HirFunctionOrigin, HirFunctionReturnContract,
};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, RegionId};
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

fn never_call_module(
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> crate::compiler_frontend::hir::module::HirModule {
    let expressions = HirExpressionStore::default();
    let halt_path = path_fork
        .try_intern_portable_path("halt", string_table)
        .expect("test path fits");
    let run_path = path_fork
        .try_intern_portable_path("run", string_table)
        .expect("test path fits");

    let halt_block = HirBlock {
        id: BlockId(10),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::RuntimeFailure {
            message: "halted".to_owned(),
            cause: None,
        },
    };
    let run_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::NeverCall {
            target: CallTarget::Local(FunctionId(1)),
            args: HirValueRange::empty(),
        },
    };
    let halt = HirFunction {
        id: FunctionId(1),
        entry: BlockId(10),
        params: vec![],
        return_contract: HirFunctionReturnContract::Never,
    };
    let run = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
            vec![],
            None,
        )),
    };

    build_module(
        path_fork,
        string_table,
        expressions,
        vec![
            (run, run_path, HirFunctionOrigin::EntryStart),
            (halt, halt_path, HirFunctionOrigin::Normal),
        ],
        vec![run_block, halt_block],
        FunctionId(0),
    )
}

/// Verifies a NeverCall lowers to a result-less call followed by Trap.
#[test]
fn never_call_lowers_to_resultless_call_then_trap() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, _) = build_type_environment();
    let module = never_call_module(&mut path_fork, &mut string_table);
    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("Wasm NeverCall lowering should succeed");

    let run = result
        .lir_module
        .functions
        .iter()
        .find(|function| function.origin == WasmLirFunctionOrigin::EntryStart)
        .expect("the start function should be lowered");
    assert_eq!(
        run.signature.results,
        Vec::new(),
        "the ordinary no-value start function keeps a zero-result Wasm signature"
    );
    assert_eq!(run.blocks.len(), 1);
    let statements = &run.blocks[0].statements;
    let WasmLirStmt::Call {
        results,
        callee,
        args,
    } = statements
        .last()
        .expect("the NeverCall must emit a call statement")
    else {
        panic!("the last statement of a NeverCall block must be a Call, got {statements:?}");
    };
    assert!(
        results.is_empty(),
        "a NeverCall has no Wasm result locals: {results:?}"
    );
    assert!(args.is_empty(), "the fixture NeverCall has no arguments");
    assert!(
        matches!(callee, WasmCalleeRef::Function(_)),
        "the NeverCall must preserve the local target identity: {callee:?}"
    );
    assert_eq!(
        run.blocks[0].terminator,
        WasmLirTerminator::Trap,
        "control after a NeverCall must use the existing trap path"
    );
}

/// Verifies an ordinary zero-success call still returns through its native channel.
#[test]
fn ordinary_zero_success_call_keeps_native_return() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, _) = build_type_environment();
    let expressions = HirExpressionStore::default();
    let helper_path = path_fork
        .try_intern_portable_path("helper", &mut string_table)
        .expect("test path fits");
    let run_path = path_fork
        .try_intern_portable_path("run", &mut string_table)
        .expect("test path fits");

    let helper_block = HirBlock {
        id: BlockId(10),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(HirValueRange::empty()),
    };
    let run_block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![statement(
            1,
            HirStatementKind::Call {
                target: CallTarget::Local(FunctionId(1)),
                args: HirValueRange::empty(),
                results: vec![],
            },
            1,
        )],
        terminator: HirTerminator::Return(HirValueRange::empty()),
    };
    let helper = HirFunction {
        id: FunctionId(1),
        entry: BlockId(10),
        params: vec![],
        return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
            vec![],
            None,
        )),
    };
    let run = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
            vec![],
            None,
        )),
    };
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![
            (run, run_path, HirFunctionOrigin::EntryStart),
            (helper, helper_path, HirFunctionOrigin::Normal),
        ],
        vec![run_block, helper_block],
        FunctionId(0),
    );

    let result = lower_hir_to_wasm_lir(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("ordinary zero-success call lowering should succeed");
    let run = result
        .lir_module
        .functions
        .iter()
        .find(|function| function.origin == WasmLirFunctionOrigin::EntryStart)
        .expect("the start function should be lowered");
    let statements = &run.blocks[0].statements;
    let WasmLirStmt::Call { results, .. } = statements
        .last()
        .expect("the ordinary call must emit a call statement")
    else {
        panic!(
            "the last statement of an ordinary zero-success call must be a Call, got {statements:?}"
        );
    };
    assert!(
        results.is_empty(),
        "ordinary zero-success calls keep an empty result list: {results:?}"
    );
    assert_eq!(
        run.blocks[0].terminator,
        WasmLirTerminator::Return { values: vec![] },
        "ordinary zero-success calls must complete through their native return, not Trap"
    );
}
