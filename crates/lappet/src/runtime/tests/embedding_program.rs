//! The tested embedding program behind the sketch in `docs/wasmtime-integration.md`.
//!
//! It reads top to bottom: engine setup, a host import, instantiation, a typed export call and
//! failure handling. It deliberately uses nothing from the shared test setup. The import and
//! call surface it uses is crate-private mechanics over plain core Wasm, not the Moth host ABI
//! and not a published embedding API.

use crate::runtime::imports::HostImports;
use crate::runtime::instance::Deadline;
use crate::runtime::{InstanceLimits, NativeOptimisation, RuntimeEngine, RuntimeError};
use wasmtime::{Val, ValType};

/// State owned by one instance. Host imports reach it and nothing else.
struct HostState {
    value: i32,
}

fn run_embedding_program(optimisation: NativeOptimisation) -> Result<(), RuntimeError> {
    // Engine setup: one native compilation policy, shared by everything built from it.
    let engine = RuntimeEngine::new(optimisation)?;

    // Compile binary core Wasm once. The module is immutable and holds no guest state.
    let wasm_bytes = wat::parse_str(include_str!("fixtures/round_trip.wat"))
        .expect("the fixture is authored, valid WAT");
    let module = engine.compile_module(&wasm_bytes)?;

    // A host import, registered by its physical Wasm signature `() -> i32`.
    let mut imports: HostImports<HostState> = HostImports::new(&engine);
    imports.define(
        "host",
        "read_value",
        [],
        [ValType::I32],
        |host_state, _arguments, results| {
            results[0] = Val::I32(host_state.value);
            Ok(())
        },
    )?;

    // Check the module's imports once. A missing or mistyped import fails here, before any
    // guest code runs.
    let prepared = imports.prepare(&module)?;

    // Instantiation: each instance owns its store, its guest and its host state.
    let mut instance = prepared.instantiate(
        HostState { value: 42 },
        InstanceLimits::default(),
        Deadline::Unbounded,
    )?;

    // A typed export call by physical Wasm signature.
    let value: i32 = instance.call("run", (), Deadline::Unbounded)?;
    assert_eq!(value, 42);

    let quotient: i32 = instance.call("divide_host_value_by", 6, Deadline::Unbounded)?;
    assert_eq!(quotient, 7);

    // Failure handling: a guest trap is a classified error, never a Moth error or a panic.
    let trapped = instance.call::<i32, i32>("divide_host_value_by", 0, Deadline::Unbounded);
    assert!(matches!(trapped, Err(RuntimeError::GuestTrap { .. })));

    // The trap quarantined the instance. It refuses further calls and can only be dropped.
    let refused = instance.call::<(), i32>("run", (), Deadline::Unbounded);
    assert!(matches!(refused, Err(RuntimeError::InstanceFaulted)));

    // Cleanup: dropping the instance frees the guest and drops its host state. A fresh
    // instance from the same prepared module starts clean.
    drop(instance);
    let mut replacement = prepared.instantiate(
        HostState { value: 9 },
        InstanceLimits::default(),
        Deadline::Unbounded,
    )?;
    let value: i32 = replacement.call("run", (), Deadline::Unbounded)?;
    assert_eq!(value, 9);

    Ok(())
}

#[test]
fn embedding_program_runs_under_both_optimisation_levels() {
    for optimisation in [NativeOptimisation::Development, NativeOptimisation::Release] {
        let outcome = run_embedding_program(optimisation);
        assert!(outcome.is_ok(), "{optimisation:?}: {:?}", outcome.err());
    }
}
