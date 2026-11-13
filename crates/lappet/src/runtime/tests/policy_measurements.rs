//! Runtime policy measurements over the authored `workloads.wat` fixture.
//!
//! These are ignored tests because they record evidence and never pass or fail on a number.
//! Correctness lives in the other test files. Run them in a release build, one at a time:
//!
//! ```text
//! cargo test -p lappet --features runtime --release -- --ignored --nocapture --test-threads=1 measure_
//! ```
//!
//! Each measurement runs under every engine configuration: both native optimisation levels,
//! with epoch interruption on and off. Set `LAPPET_MEASURE_CONFIGURATION` to one configuration
//! label, such as `release/epoch-on`, to run only that one. A process that runs one
//! configuration of one workload can then be wrapped in an external peak-memory measurement.
//!
//! Every timed operation is repeated and reported as its fastest and median repetition. Cold
//! and reused states are never mixed: a cold compilation builds a fresh engine each time, and
//! the other measurements reuse one engine, one compiled module and one prepared module.

use super::assemble;
use crate::runtime::engine::EpochInterruption;
use crate::runtime::imports::HostImports;
use crate::runtime::instance::{Deadline, Instance, PreparedModule};
use crate::runtime::{InstanceLimits, NativeOptimisation, RuntimeEngine, RuntimeError};
use std::time::{Duration, Instant};
use wasmtime::{Val, ValType};

/// One engine policy under measurement.
#[derive(Debug, Clone, Copy)]
struct EngineConfiguration {
    label: &'static str,
    optimisation: NativeOptimisation,
    epoch_interruption: EpochInterruption,
}

const ENGINE_CONFIGURATIONS: [EngineConfiguration; 4] = [
    EngineConfiguration {
        label: "development/epoch-on",
        optimisation: NativeOptimisation::Development,
        epoch_interruption: EpochInterruption::Enabled,
    },
    EngineConfiguration {
        label: "development/epoch-off",
        optimisation: NativeOptimisation::Development,
        epoch_interruption: EpochInterruption::Disabled,
    },
    EngineConfiguration {
        label: "release/epoch-on",
        optimisation: NativeOptimisation::Release,
        epoch_interruption: EpochInterruption::Enabled,
    },
    EngineConfiguration {
        label: "release/epoch-off",
        optimisation: NativeOptimisation::Release,
        epoch_interruption: EpochInterruption::Disabled,
    },
];

/// The configurations this process measures: all of them, or the one the environment names.
fn selected_configurations() -> Vec<EngineConfiguration> {
    let Ok(selected_label) = std::env::var("LAPPET_MEASURE_CONFIGURATION") else {
        return ENGINE_CONFIGURATIONS.to_vec();
    };

    let selected: Vec<EngineConfiguration> = ENGINE_CONFIGURATIONS
        .into_iter()
        .filter(|configuration| configuration.label == selected_label)
        .collect();
    assert!(
        !selected.is_empty(),
        "LAPPET_MEASURE_CONFIGURATION names no configuration: {selected_label}"
    );
    selected
}

/// Host state for the workload imports.
struct WorkloadHost {
    /// Returned by `host.read_input`, indexed by its argument.
    inputs: [i32; 3],
    /// The deepest level `host.record_depth` saw.
    deepest_recursion: i32,
}

impl WorkloadHost {
    fn new() -> Self {
        Self {
            inputs: [3, 5, 7],
            deepest_recursion: 0,
        }
    }
}

fn workload_imports(engine: &RuntimeEngine) -> Result<HostImports<WorkloadHost>, RuntimeError> {
    let mut imports = HostImports::new(engine);

    imports.define(
        "host",
        "read_input",
        [ValType::I32],
        [ValType::I32],
        |host: &mut WorkloadHost, _staging, _limits, arguments, results| {
            let index = arguments[0].i32().expect("the import takes one i32");
            results[0] = Val::I32(host.inputs[index.unsigned_abs() as usize % host.inputs.len()]);
            Ok(())
        },
    )?;
    imports.define(
        "host",
        "record_depth",
        [ValType::I32],
        [],
        |host: &mut WorkloadHost, _staging, _limits, arguments, _results| {
            host.deepest_recursion = arguments[0].i32().expect("the import takes one i32");
            Ok(())
        },
    )?;

    Ok(imports)
}

/// One engine with the workload module compiled and its imports checked, ready to reuse.
struct WorkloadRuntime {
    prepared: PreparedModule<WorkloadHost>,
}

impl WorkloadRuntime {
    fn new(configuration: EngineConfiguration) -> Self {
        let engine = new_engine(configuration);
        let module = engine
            .compile_module(&workload_bytes())
            .expect("the workload fixture is valid core Wasm");
        let prepared = workload_imports(&engine)
            .and_then(|imports| imports.prepare(&module))
            .expect("the workload host provides every import");
        Self { prepared }
    }

    fn instantiate(&self) -> Instance<WorkloadHost> {
        self.prepared
            .instantiate(
                WorkloadHost::new(),
                InstanceLimits::default(),
                Deadline::Unbounded,
            )
            .expect("the workload fixture has no start function and fits the limits")
    }
}

fn new_engine(configuration: EngineConfiguration) -> RuntimeEngine {
    RuntimeEngine::with_epoch_interruption(
        configuration.optimisation,
        configuration.epoch_interruption,
    )
    .expect("the engine policy is supported on the measuring host")
}

fn workload_bytes() -> Vec<u8> {
    assemble(include_str!("fixtures/workloads.wat"))
}

/// The fastest and the median repetition of one timed operation.
struct Timing {
    fastest: Duration,
    median: Duration,
}

/// Times `operation` alone. `prepare` builds its input outside the timed region, so setup and
/// the drop of the operation's result never count.
fn time_repetitions<Input, Output>(
    repetitions: usize,
    mut prepare: impl FnMut() -> Input,
    mut operation: impl FnMut(Input) -> Output,
) -> Timing {
    let mut durations = Vec::with_capacity(repetitions);
    for _ in 0..repetitions {
        let input = prepare();
        let started = Instant::now();
        let output = operation(input);
        durations.push(started.elapsed());
        drop(output);
    }

    durations.sort_unstable();
    Timing {
        fastest: durations[0],
        median: durations[repetitions / 2],
    }
}

fn report(measurement: &str, configuration: EngineConfiguration, timing: &Timing, per_unit: u32) {
    println!(
        "{measurement:<34} {:<22} fastest {:>12}  median {:>12}",
        configuration.label,
        format_per_unit(timing.fastest, per_unit),
        format_per_unit(timing.median, per_unit),
    );
}

/// Divides in floating point. `Duration` division truncates to a whole nanosecond, which hides a
/// per-iteration cost below that.
fn format_per_unit(duration: Duration, per_unit: u32) -> String {
    let nanoseconds = duration.as_secs_f64() * 1_000_000_000.0 / f64::from(per_unit);
    if nanoseconds < 1_000.0 {
        format!("{nanoseconds:.1} ns")
    } else if nanoseconds < 1_000_000.0 {
        format!("{:.1} us", nanoseconds / 1_000.0)
    } else if nanoseconds < 1_000_000_000.0 {
        format!("{:.1} ms", nanoseconds / 1_000_000.0)
    } else {
        format!("{:.2} s", nanoseconds / 1_000_000_000.0)
    }
}

#[test]
#[ignore = "records policy evidence; run in a release build with --ignored"]
fn measure_setup_and_teardown() {
    let wasm_bytes = workload_bytes();

    for configuration in selected_configurations() {
        let engine_setup = time_repetitions(30, || (), |()| new_engine(configuration));
        report("engine setup", configuration, &engine_setup, 1);

        // Cold: a fresh engine for every compilation, built outside the timed region.
        let cold_compilation = time_repetitions(
            30,
            || new_engine(configuration),
            |engine| {
                let module = engine.compile_module(&wasm_bytes);
                (engine, module)
            },
        );
        report(
            "cold module compilation",
            configuration,
            &cold_compilation,
            1,
        );

        // Reused from here on: one engine and one compiled module.
        let engine = new_engine(configuration);
        let module = engine
            .compile_module(&wasm_bytes)
            .expect("the workload fixture is valid core Wasm");
        let imports = workload_imports(&engine).expect("each import is defined once");
        // The timed samples are reused state only. The first call is outside the timer.
        imports
            .prepare(&module)
            .expect("the workload host provides every import");
        let import_preflight = time_repetitions(1_000, || (), |()| imports.prepare(&module));
        report("import preflight", configuration, &import_preflight, 1);

        let runtime = WorkloadRuntime::new(configuration);
        drop(runtime.instantiate());
        let instantiation = time_repetitions(2_000, || (), |()| runtime.instantiate());
        report("instantiation", configuration, &instantiation, 1);

        drop(runtime.instantiate());
        let teardown = time_repetitions(2_000, || runtime.instantiate(), drop);
        report("teardown", configuration, &teardown, 1);
    }
}

#[test]
#[ignore = "records policy evidence; run in a release build with --ignored"]
fn measure_game_workload() {
    const ENTITIES_PER_FRAME: u32 = 1_000;

    for configuration in selected_configurations() {
        let runtime = WorkloadRuntime::new(configuration);
        // One long-lived instance, as a game keeps one gameplay instance per world.
        let mut instance = runtime.instantiate();

        let frame = time_repetitions(
            300,
            || (),
            |()| {
                for entity in 0..ENTITIES_PER_FRAME {
                    instance
                        .call::<i32, i32>("step_entity", entity as i32, Deadline::Unbounded)
                        .expect("the step has no fault path");
                }
            },
        );
        report("game frame, 1000 entity steps", configuration, &frame, 1);
        report(
            "game entity step",
            configuration,
            &frame,
            ENTITIES_PER_FRAME,
        );
    }
}

#[test]
#[ignore = "records policy evidence; run in a release build with --ignored"]
fn measure_server_workload() {
    const PAGES_PER_REQUEST: i32 = 16;

    for configuration in selected_configurations() {
        let runtime = WorkloadRuntime::new(configuration);

        // Teardown is timed on its own. The instance is returned so this helper drops it
        // after the sample.
        let request = time_repetitions(
            500,
            || (),
            |()| {
                let mut instance = runtime.instantiate();
                instance
                    .call::<i32, i32>("handle_request", PAGES_PER_REQUEST, Deadline::Unbounded)
                    .expect("one mebibyte fits the default memory ceiling");
                instance
            },
        );
        report("server request, 1 MiB written", configuration, &request, 1);
    }
}

#[test]
#[ignore = "records policy evidence; run in a release build with --ignored"]
fn measure_compute_and_host_call_cost() {
    const COLLATZ_LIMIT: i64 = 100_000;
    const LOOP_ITERATIONS: u32 = 1_000_000;

    for configuration in selected_configurations() {
        let runtime = WorkloadRuntime::new(configuration);
        let mut instance = runtime.instantiate();

        let compute = time_repetitions(
            20,
            || (),
            |()| {
                instance
                    .call::<i64, i64>("sum_collatz_steps", COLLATZ_LIMIT, Deadline::Unbounded)
                    .expect("the loop has no fault path")
            },
        );
        report("compute, Collatz to 100000", configuration, &compute, 1);

        // The host-call cost is the difference between these two loops, per iteration.
        for export_name in ["loop_with_host_call", "loop_without_host_call"] {
            let counted_loop = time_repetitions(
                20,
                || (),
                |()| {
                    instance
                        .call::<i32, i32>(export_name, LOOP_ITERATIONS as i32, Deadline::Unbounded)
                        .expect("the loop has no fault path")
                },
            );
            report(export_name, configuration, &counted_loop, LOOP_ITERATIONS);
        }
    }
}

#[test]
#[ignore = "records policy evidence; run in a release build with --ignored"]
fn measure_recursion_depth_before_the_stack_limit() {
    for configuration in selected_configurations() {
        let runtime = WorkloadRuntime::new(configuration);
        let mut instance = runtime.instantiate();

        // The call ends in the stack fault. Host state stays readable afterwards.
        let fault = instance.call::<i32, ()>("descend", 0, Deadline::Unbounded);
        println!(
            "{:<34} {:<22} depth {} then {}",
            "recursion before stack limit",
            configuration.label,
            instance.host_state().deepest_recursion,
            fault.expect_err("unbounded recursion always faults"),
        );
    }
}
