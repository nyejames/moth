//! Runtime mechanics tests over hand-authored core Wasm fixtures.
//!
//! The fixtures under `fixtures/` prove Wasmtime integration only. They say nothing about the
//! Moth host ABI or compiled Moth output. Every behaviour runs under both native optimisation
//! levels, because the levels compile the same Wasm through different Cranelift pipelines.
//!
//! `embedding_program.rs` is the self-contained program the Wasmtime reference links to. The
//! other files share the `guest.wat` fixture and the [`TestHost`] state defined here.

mod embedding_program;
mod engine_tests;
mod fault_tests;
mod instance_tests;
mod limit_tests;

use crate::runtime::imports::HostImports;
use crate::runtime::instance::{Deadline, Instance, PreparedModule};
use crate::runtime::{InstanceLimits, NativeOptimisation, RuntimeEngine, RuntimeError};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use wasmtime::{Val, ValType};

/// Both native optimisation levels, so a failure names the level that broke.
const BOTH_OPTIMISATIONS: [NativeOptimisation; 2] =
    [NativeOptimisation::Development, NativeOptimisation::Release];

/// Assembles a WAT fixture into the binary Wasm the runtime compiles.
fn assemble(wat_source: &str) -> Vec<u8> {
    wat::parse_str(wat_source).expect("the fixture is authored, valid WAT")
}

fn engine(optimisation: NativeOptimisation) -> Arc<RuntimeEngine> {
    let engine =
        RuntimeEngine::new(optimisation).expect("the engine policy is supported on a test host");
    Arc::new(engine)
}

/// Per-instance host state for the `guest.wat` imports.
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

/// One engine with the `guest.wat` module prepared against the test host imports.
struct GuestHarness {
    engine: Arc<RuntimeEngine>,
    prepared: PreparedModule<TestHost>,
    drop_count: Arc<AtomicUsize>,
}

impl GuestHarness {
    fn new(optimisation: NativeOptimisation) -> Self {
        let engine = engine(optimisation);
        let module = engine
            .compile_module(&assemble(include_str!("fixtures/guest.wat")))
            .expect("the guest fixture is valid core Wasm");
        let prepared = test_host_imports(&engine)
            .prepare(&module)
            .expect("the test host provides every guest import");

        Self {
            engine,
            prepared,
            drop_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn instantiate(&self, value: i32) -> Instance<TestHost> {
        self.instantiate_with_limits(value, InstanceLimits::default())
    }

    fn instantiate_with_limits(&self, value: i32, limits: InstanceLimits) -> Instance<TestHost> {
        let host_state = TestHost {
            value,
            recorded: Vec::new(),
            engine: Arc::clone(&self.engine),
            drop_count: Arc::clone(&self.drop_count),
        };
        self.prepared
            .instantiate(host_state, limits, Deadline::Unbounded)
            .expect("the guest fixture has no start function and fits the limits")
    }

    fn host_states_dropped(&self) -> usize {
        self.drop_count.load(Ordering::SeqCst)
    }
}

/// The host side of every `guest.wat` import.
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

    // The three ways a host import can fault.
    imports.define("host", "fail", [], [], |_host, _arguments, _results| {
        Err("the disk is full".into())
    })?;
    imports.define("host", "panic", [], [], |_host, _arguments, _results| {
        panic!("host import bug")
    })?;
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
