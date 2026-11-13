//! Runtime mechanics tests over hand-authored core Wasm fixtures.
//!
//! The fixtures under `fixtures/` prove Wasmtime integration only. They say nothing about the
//! Moth host ABI or compiled Moth output. Every behaviour runs under both native optimisation
//! levels through [`under_both_optimisations`], because the levels compile the same Wasm
//! through different Cranelift pipelines.
//!
//! `embedding_program.rs` is the self-contained program the Wasmtime reference links to. The
//! other files share the `guest.wat` fixture and the [`TestHost`] state defined here.

mod embedding_program;
mod engine_tests;
mod fault_tests;
mod instance_tests;
mod limit_tests;
mod policy_measurements;
mod staging_tests;

use crate::runtime::imports::HostImports;
use crate::runtime::instance::{Deadline, Instance, PreparedModule};
use crate::runtime::{InstanceLimits, NativeOptimisation, RuntimeEngine, RuntimeError};
use std::panic::panic_any;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use wasmtime::{Val, ValType};

/// Runs one scenario under each native optimisation level.
///
/// Any panic inside the scenario, including a setup `expect`, is followed by a line naming the
/// level it ran under, so no assertion has to carry that context itself.
fn under_both_optimisations(scenario: impl Fn(NativeOptimisation)) {
    for optimisation in [NativeOptimisation::Development, NativeOptimisation::Release] {
        let _level_on_panic = LevelOnPanic(optimisation);
        scenario(optimisation);
    }
}

struct LevelOnPanic(NativeOptimisation);

impl Drop for LevelOnPanic {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("the scenario failed under native optimisation {:?}", self.0);
        }
    }
}

/// Assembles a WAT fixture into the binary Wasm the runtime compiles. The assembler doesn't
/// validate, so a fixture may be deliberately invalid Wasm.
fn assemble(wat_source: &str) -> Vec<u8> {
    wat::parse_str(wat_source).expect("the fixture is authored, well-formed WAT")
}

fn engine(optimisation: NativeOptimisation) -> Arc<RuntimeEngine> {
    let engine =
        RuntimeEngine::new(optimisation).expect("the engine policy is supported on a test host");
    Arc::new(engine)
}

/// Per-instance host state for the test host imports.
struct TestHost {
    /// Returned by `host.read_value`.
    value: i32,
    /// Every argument `host.record` received, in call order.
    recorded: Vec<i32>,
    /// Lets `host.advance_epoch` act as the embedder's clock from inside a guest call, so
    /// deadline tests need no second thread.
    engine: Arc<RuntimeEngine>,
    /// Counts drops of this state, to prove teardown releases it exactly once.
    drop_count: Arc<AtomicUsize>,
}

impl Drop for TestHost {
    fn drop(&mut self) {
        self.drop_count.fetch_add(1, Ordering::SeqCst);
    }
}

/// One engine with the test host imports and the `guest.wat` module prepared against them.
struct GuestHarness {
    engine: Arc<RuntimeEngine>,
    imports: HostImports<TestHost>,
    guest: PreparedModule<TestHost>,
    drop_count: Arc<AtomicUsize>,
}

impl GuestHarness {
    fn new(optimisation: NativeOptimisation) -> Self {
        let engine = engine(optimisation);
        let imports = test_host_imports(&engine);
        let guest = prepare(&engine, &imports, include_str!("fixtures/guest.wat"))
            .expect("the test host provides every guest import");

        Self {
            engine,
            imports,
            guest,
            drop_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Prepares another module against the same engine and test host imports.
    fn prepare(&self, wat_source: &str) -> Result<PreparedModule<TestHost>, RuntimeError> {
        prepare(&self.engine, &self.imports, wat_source)
    }

    fn host_state(&self, value: i32) -> TestHost {
        TestHost {
            value,
            recorded: Vec::new(),
            engine: Arc::clone(&self.engine),
            drop_count: Arc::clone(&self.drop_count),
        }
    }

    fn instantiate(&self, value: i32) -> Instance<TestHost> {
        self.instantiate_with_limits(value, InstanceLimits::default())
    }

    fn instantiate_with_limits(&self, value: i32, limits: InstanceLimits) -> Instance<TestHost> {
        self.guest
            .instantiate(self.host_state(value), limits, Deadline::Unbounded)
            .expect("the guest fixture has no start function and fits the limits")
    }

    fn host_states_dropped(&self) -> usize {
        self.drop_count.load(Ordering::SeqCst)
    }
}

/// Compiles a WAT module and checks its imports against `imports`.
fn prepare<HostState: 'static>(
    engine: &RuntimeEngine,
    imports: &HostImports<HostState>,
    wat_source: &str,
) -> Result<PreparedModule<HostState>, RuntimeError> {
    let module = engine.compile_module(&assemble(wat_source))?;
    imports.prepare(&module)
}

/// A panic payload whose own destructor panics.
struct PanicsWhenDropped;

impl Drop for PanicsWhenDropped {
    fn drop(&mut self) {
        panic!("the panic payload's destructor panicked");
    }
}

/// The host side of every test import.
fn test_host_imports(engine: &RuntimeEngine) -> HostImports<TestHost> {
    let mut imports = HostImports::new(engine);
    define_test_host_imports(&mut imports).expect("each test import name is defined once");
    imports
}

fn define_test_host_imports(imports: &mut HostImports<TestHost>) -> Result<(), RuntimeError> {
    imports.define(
        "host",
        "read_value",
        [],
        [ValType::I32],
        |host, _arguments, results| {
            results[0] = Val::I32(host.value);
            Ok(())
        },
    )?;
    imports.define(
        "host",
        "record",
        [ValType::I32],
        [],
        |host, arguments, _results| {
            let value = arguments[0].i32().expect("the import takes one i32");
            host.recorded.push(value);
            Ok(())
        },
    )?;
    imports.define(
        "host",
        "advance_epoch",
        [],
        [],
        |host, _arguments, _results| {
            host.engine.advance_epoch();
            Ok(())
        },
    )?;

    // The ways a host import can fault.
    imports.define("host", "fail", [], [], |_host, _arguments, _results| {
        Err("the disk is full".into())
    })?;
    imports.define("host", "panic", [], [], |_host, _arguments, _results| {
        panic!("host import bug")
    })?;
    imports.define(
        "host",
        "panic_with_payload_that_panics_when_dropped",
        [],
        [],
        |_host, _arguments, _results| panic_any(PanicsWhenDropped),
    )?;
    imports.define(
        "host",
        "wrong_result_type",
        [],
        [ValType::I32],
        |_host, _arguments, results| {
            results[0] = Val::I64(7);
            Ok(())
        },
    )
}
