//! Engine construction, module compilation and engine identity.

use super::{assemble, engine, test_host_imports, under_both_optimisations};
use crate::runtime::RuntimeError;

#[test]
fn valid_module_compiles() {
    let wasm_bytes = assemble(include_str!("fixtures/guest.wat"));

    under_both_optimisations(|optimisation| {
        let compiled = engine(optimisation).compile_module(&wasm_bytes);
        assert!(compiled.is_ok(), "{:?}", compiled.err());
    });
}

#[test]
fn bytes_that_are_not_a_valid_module_are_rejected_as_invalid() {
    let complete = assemble("(module)");
    let truncated = &complete[..complete.len() - 1];
    // Well-formed binary whose function body doesn't produce its declared result.
    let fails_validation = assemble("(module (func (result i32)))");
    let rejected_inputs: [(&str, &[u8]); 4] = [
        ("truncated header", truncated),
        ("text instead of binary", b"(module)"),
        ("empty input", b""),
        ("function body fails validation", &fails_validation),
    ];

    under_both_optimisations(|optimisation| {
        let engine = engine(optimisation);
        for (label, wasm_bytes) in rejected_inputs {
            let result = engine.compile_module(wasm_bytes);
            assert!(
                matches!(result, Err(RuntimeError::InvalidModule { .. })),
                "{label}"
            );
        }
    });
}

#[test]
fn module_needing_a_proposal_outside_the_profile_is_rejected() {
    // Memory64, custom page sizes and relaxed SIMD are switched off explicitly. Shared memories
    // need the threads proposal, which isn't compiled into the dependency.
    let disabled_proposals = [
        ("memory64", "(module (memory i64 1))"),
        ("custom page sizes", "(module (memory 1 (pagesize 1)))"),
        (
            "relaxed SIMD",
            "(module (func (param v128 v128 v128) (result v128)
               (f32x4.relaxed_madd (local.get 0) (local.get 1) (local.get 2))))",
        ),
        ("threads", "(module (memory 1 1 shared))"),
    ];

    under_both_optimisations(|optimisation| {
        let engine = engine(optimisation);
        for (proposal, wat_source) in disabled_proposals {
            let result = engine.compile_module(&assemble(wat_source));
            assert!(
                matches!(result, Err(RuntimeError::InvalidModule { .. })),
                "{proposal}"
            );
        }
    });
}

#[test]
fn multi_value_results_are_part_of_the_profile() {
    let wasm_bytes = assemble("(module (func (result i32 i64) i32.const 1 i64.const 2))");

    under_both_optimisations(|optimisation| {
        let compiled = engine(optimisation).compile_module(&wasm_bytes);
        assert!(compiled.is_ok(), "{:?}", compiled.err());
    });
}

#[test]
fn module_used_with_another_engine_is_a_classified_error() {
    let wasm_bytes = assemble(include_str!("fixtures/guest.wat"));

    under_both_optimisations(|optimisation| {
        let compiling_engine = engine(optimisation);
        let other_engine = engine(optimisation);
        let module = compiling_engine
            .compile_module(&wasm_bytes)
            .expect("the guest fixture is valid core Wasm");

        let with_other_engine = test_host_imports(&other_engine).prepare(&module);
        assert!(matches!(
            with_other_engine,
            Err(RuntimeError::EngineMismatch)
        ));

        // The same module still links against the engine that compiled it.
        let with_compiling_engine = test_host_imports(&compiling_engine).prepare(&module);
        assert!(with_compiling_engine.is_ok());
    });
}
