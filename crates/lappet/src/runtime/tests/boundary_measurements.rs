//! Boundary-table policy measurements.
//!
//! Ignored tests. They record evidence and never pass or fail on a number. Run them in a
//! release build:
//!
//! ```text
//! cargo test -p lappet --features runtime --release -- --ignored --nocapture --test-threads=1 measure_
//! ```
//!
//! Staging and the resource table are host-side. Their cost does not depend on Cranelift, so
//! those two measurements use one engine. The guard measurement calls into a guest and runs
//! under both native optimisation levels.

use super::{assemble, engine};
use crate::runtime::imports::HostImports;
use crate::runtime::instance::{Deadline, Instance};
use crate::runtime::resource::AccessRequest;
use crate::runtime::staging::StagingTable;
use crate::runtime::{InstanceLimits, NativeOptimisation};
use std::time::{Duration, Instant};
use wasmtime::ValType;

const SMALL_TEXT: usize = 16;
const ENTRY_LIMIT: usize = 16 * 1024 * 1024;
const LIVE_LIMIT: usize = 1_048_576;

#[test]
#[ignore = "records policy evidence; run in a release build with --ignored"]
fn measure_staging_cost() {
    let limits = InstanceLimits::default();
    let small = vec![b'a'; SMALL_TEXT];
    let limited = vec![b'a'; ENTRY_LIMIT];
    let small_timing = time_stage(&small, &limits, 40);
    let limited_timing = time_stage(&limited, &limits, 5);
    println!(
        "stage {SMALL_TEXT} bytes          median {}",
        format_duration(small_timing)
    );
    println!(
        "stage {ENTRY_LIMIT} bytes   median {}",
        format_duration(limited_timing)
    );
    println!(
        "one limit-sized entry holds {ENTRY_LIMIT} bytes live, under the {} byte live ceiling",
        limits.max_staged_live_bytes
    );
}

#[test]
#[ignore = "records policy evidence; run in a release build with --ignored"]
fn measure_guard_cost_per_host_call() {
    for optimisation in [NativeOptimisation::Development, NativeOptimisation::Release] {
        let mut instance = guard_instance(optimisation);
        let token = instance.insert_resource(1u32).expect("the table has room");
        let bare = time_calls(2_000, || {
            instance
                .call::<(), ()>("bare", (), Deadline::Unbounded)
                .expect("the bare call has no fault path");
        });
        let guarded = time_calls(2_000, || {
            instance
                .call::<i64, ()>("guarded", token.raw(), Deadline::Unbounded)
                .expect("the guarded call has no fault path");
        });
        let label = match optimisation {
            NativeOptimisation::Development => "development",
            NativeOptimisation::Release => "release",
        };
        println!(
            "host call {label:<12} bare median {}  guarded median {}  difference {}",
            format_duration(bare),
            format_duration(guarded),
            format_duration(guarded.saturating_sub(bare))
        );
    }
}

#[test]
#[ignore = "records policy evidence; run in a release build with --ignored"]
fn measure_resource_table_at_the_live_limit() {
    let mut instance = empty_instance();
    let before = resident_kib();
    let mut samples = Vec::new();
    for index in 0..LIVE_LIMIT {
        let token = instance
            .insert_resource(index as u32)
            .expect("the live-entry ceiling holds this entry");
        if index.is_multiple_of(LIVE_LIMIT / 64) {
            samples.push(token);
        }
    }
    let after = resident_kib();
    let lookups = samples.len() as u32 * 20;
    let lookup = time_calls(1, || {
        for _ in 0..20 {
            for token in &samples {
                let _ = instance.resource(*token).expect("a live token resolves");
            }
        }
    });
    let per_entry = after.saturating_sub(before) as f64 * 1024.0 / LIVE_LIMIT as f64;
    println!(
        "live entries {LIVE_LIMIT}  resident {before} KiB to {after} KiB  {per_entry:.1} bytes per live entry"
    );
    println!(
        "lookup of {} spread tokens, 20 passes, median {}",
        samples.len(),
        format_duration(lookup / lookups)
    );
    let closed = samples[0];
    instance
        .close_resource(closed)
        .expect("close returns the value and removes the entry");
    let _ = instance
        .insert_resource(0u32)
        .expect("a closed entry does not keep its slot");
}

fn time_stage(bytes: &[u8], limits: &InstanceLimits, repetitions: usize) -> Duration {
    let mut table = StagingTable::new();
    let mut durations = Vec::with_capacity(repetitions);
    for _ in 0..repetitions {
        let started = Instant::now();
        let ticket = table
            .stage(bytes, limits)
            .expect("the entry fits the ceilings");
        durations.push(started.elapsed());
        table
            .take(ticket, bytes.len())
            .expect("the staged ticket is live");
    }
    durations.sort_unstable();
    durations[repetitions / 2]
}

fn time_calls(repetitions: usize, mut call: impl FnMut()) -> Duration {
    let mut durations = Vec::with_capacity(repetitions);
    for _ in 0..repetitions {
        let started = Instant::now();
        call();
        durations.push(started.elapsed());
    }
    durations.sort_unstable();
    durations[repetitions / 2]
}

fn guard_instance(optimisation: NativeOptimisation) -> Instance<GuardHost> {
    let engine = engine(optimisation);
    let mut imports = HostImports::new(&engine);
    imports
        .define(
            "host",
            "bare",
            [],
            [],
            |_: &mut GuardHost, _, _, _, _, _, _| Ok(()),
        )
        .expect("the bare import is defined once");
    imports
        .define(
            "host",
            "guarded",
            [ValType::I64],
            [],
            |_: &mut GuardHost, _, _, resources, _, arguments, _| {
                let raw = arguments[0].i64().expect("the import takes one i64 token");
                resources
                    .with_access(
                        &[AccessRequest {
                            raw,
                            exclusive: false,
                            type_id: std::any::TypeId::of::<u32>(),
                        }],
                        |access| {
                            let _ = access.shared::<u32>(0);
                        },
                    )
                    .map_err(|fault| -> crate::runtime::error::ErrorSource { Box::new(fault) })?;
                Ok(())
            },
        )
        .expect("the guarded import is defined once");
    let module = engine
        .compile_module(&assemble(include_str!("fixtures/guard_cost.wat")))
        .expect("the fixture is valid core Wasm");
    imports
        .prepare(&module)
        .expect("the fixture imports are provided")
        .instantiate(GuardHost, InstanceLimits::default(), Deadline::Unbounded)
        .expect("the fixture has no start function")
}

fn empty_instance() -> Instance<()> {
    let engine = engine(NativeOptimisation::Release);
    let imports = HostImports::<()>::new(&engine);
    let module = engine
        .compile_module(&assemble("(module)"))
        .expect("an empty module is valid core Wasm");
    imports
        .prepare(&module)
        .expect("an empty module has no imports")
        .instantiate((), InstanceLimits::default(), Deadline::Unbounded)
        .expect("an empty module has no start function")
}

struct GuardHost;

fn resident_kib() -> u64 {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps reports the resident set");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("ps prints the resident set in KiB")
}

fn format_duration(duration: Duration) -> String {
    let nanoseconds = duration.as_secs_f64() * 1_000_000_000.0;
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
