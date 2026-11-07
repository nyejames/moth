//! Never-call Wasm emission artefact tests.
//!
//! WHAT: proves the emitted module validates and that a NeverCall body contains `call` then
//!       `unreachable` in that order.
//! WHY: LIR Trap must remain the existing unrecoverable path; emission must not drop the call or
//!      invent a redesigned terminator.

use crate::backends::wasm::backend::lower_hir_to_wasm_module;
use crate::backends::wasm::request::WasmBackendRequest;
use crate::backends::wasm::tests::lowering::test_support::{
    build_module, build_type_environment, default_borrow_facts, default_numeric_proofs,
};
use crate::compiler_frontend::datatypes::CallableReturnShape;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expression_store::{HirExpressionStore, HirValueRange};
use crate::compiler_frontend::hir::functions::{
    HirFunction, HirFunctionOrigin, HirFunctionReturnContract,
};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, RegionId};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use wasmparser::{Operator, Parser, Payload};

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

fn function_operators(bytes: &[u8]) -> Vec<Vec<String>> {
    let mut bodies = Vec::new();
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.expect("emitted Wasm should parse");
        let Payload::CodeSectionEntry(body) = payload else {
            continue;
        };
        let mut reader = body
            .get_operators_reader()
            .expect("function body should expose operators");
        let mut operators = Vec::new();
        while !reader.eof() {
            let operator = reader.read().expect("operator should parse");
            operators.push(match operator {
                Operator::Call { function_index } => format!("call {function_index}"),
                Operator::Unreachable => "unreachable".to_owned(),
                Operator::Return => "return".to_owned(),
                Operator::End => "end".to_owned(),
                other => format!("{other:?}"),
            });
        }
        bodies.push(operators);
    }
    bodies
}

/// Verifies emitted NeverCall Wasm validates and contains `call` then `unreachable`.
#[test]
fn never_call_wasm_validates_and_contains_call_then_unreachable() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, _) = build_type_environment();
    let module = never_call_module(&mut path_fork, &mut string_table);
    let result = lower_hir_to_wasm_module(
        &module,
        &default_borrow_facts(),
        &default_numeric_proofs(),
        &WasmBackendRequest::default(),
        &string_table,
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("Wasm NeverCall emission should succeed");
    let wasm_bytes = result.wasm_bytes.expect("Wasm bytes should be present");
    wasmparser::Validator::new()
        .validate_all(&wasm_bytes)
        .expect("emitted NeverCall Wasm should validate");

    let bodies = function_operators(&wasm_bytes);
    assert!(
        bodies.iter().any(|operators| {
            operators
                .windows(2)
                .any(|pair| pair[0].starts_with("call ") && pair[1] == "unreachable")
        }),
        "at least one function body must contain call then unreachable: {bodies:?}"
    );
}
