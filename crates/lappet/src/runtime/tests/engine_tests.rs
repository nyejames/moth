//! Engine construction, module compilation and engine identity.

use super::{BOTH_OPTIMISATIONS, TestHost, assemble, engine, test_host_imports};
use crate::runtime::RuntimeError;

#[test]
fn valid_module_compiles_under_both_optimisation_levels() {
    let wasm_bytes = assemble(include_str!("fixtures/guest.wat"));

    for optimisation in BOTH_OPTIMISATIONS {
        let compiled = engine(optimisation).compile_module(&wasm_bytes);
        assert!(compiled.is_ok(), "{optimisation:?}: {:?}", compiled.err());
    }
}

#[test]
fn bytes_that_are_not_a_module_are_rejected_as_invalid() {
    let complete = assemble("(module)");
    let truncated = &complete[..complete.len() - 1];
    let rejected_inputs: [(&str, &[u8]); 3] = [
        ("truncated header", truncated),
        ("text instead of binary", b"(module)"),
        ("empty input", b""),
    ];

    for optimisation in BOTH_OPTIMISATIONS {
        let engine = engine(optimisation);
        for (label, wasm_bytes) in rejected_inputs {
            let result = engine.compile_module(wasm_bytes);
            assert!(
                matches!(result, Err(RuntimeError::InvalidModule { .. })),
                "{optimisation:?}, {label}"
            );
        }
    }
}

#[test]
fn module_needing_a_disabled_proposal_is_rejected() {
    // Memory64 and custom page sizes are switched off explicitly. Shared memories need the
    // threads proposal, which isn't compiled into the dependency.
    let disabled_proposals = [
        ("memory64", "(module (memory i64 1))"),
        ("custom page sizes", "(module (memory 1 (pagesize 1)))"),
        ("threads", "(module (memory 1 1 shared))"),
    ];

    for optimisation in BOTH_OPTIMISATIONS {
        let engine = engine(optimisation);
        for (proposal, wat_source) in disabled_proposals {
            let result = engine.compile_module(&assemble(wat_source));
            assert!(
                matches!(result, Err(RuntimeError::InvalidModule { .. })),
                "{optimisation:?}, {proposal}"
            );
        }
    }
}

#[test]
fn multi_value_results_are_part_of_the_profile() {
    let wasm_bytes = assemble("(module (func (result i32 i64) i32.const 1 i64.const 2))");

    for optimisation in BOTH_OPTIMISATIONS {
        let compiled = engine(optimisation).compile_module(&wasm_bytes);
        assert!(compiled.is_ok(), "{optimisation:?}: {:?}", compiled.err());
    }
}

#[test]
fn module_used_with_another_engine_is_a_classified_error() {
    let wasm_bytes = assemble(include_str!("fixtures/guest.wat"));

    for optimisation in BOTH_OPTIMISATIONS {
        let compiling_engine = engine(optimisation);
        let other_engine = engine(optimisation);
        let module = compiling_engine
            .compile_module(&wasm_bytes)
            .expect("the guest fixture is valid core Wasm");

        let result = test_host_imports(&other_engine).prepare(&module);

        assert!(
            matches!(result, Err(RuntimeError::EngineMismatch)),
            "{optimisation:?}"
        );
        // The same module still links against the engine that compiled it.
        let _prepared: crate::runtime::instance::PreparedModule<TestHost> =
            test_host_imports(&compiling_engine)
                .prepare(&module)
                .expect("the compiling engine accepts its own module");
    }
}
