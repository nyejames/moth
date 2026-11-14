//! Resource limits and execution deadlines.

use super::{GuestHarness, prepare, under_both_optimisations};
use crate::runtime::imports::HostImports;
use crate::runtime::instance::{Deadline, Instance};
use crate::runtime::{InstanceLimits, LimitFault, NativeOptimisation, RuntimeError};

const WASM_PAGE_BYTES: usize = 65_536;

/// Instantiates an import-free module with no host state under the given limits.
fn instantiate_bare(
    optimisation: NativeOptimisation,
    wat_source: &str,
    limits: InstanceLimits,
) -> Result<Instance<()>, RuntimeError> {
    let engine = super::engine(optimisation);
    let prepared = prepare(&engine, &HostImports::new(&engine), wat_source)?;

    prepared.instantiate((), limits, Deadline::Unbounded)
}

#[test]
fn module_that_cannot_fit_the_limits_is_rejected() {
    let two_pages_one_element = InstanceLimits {
        max_memory_bytes: 2 * WASM_PAGE_BYTES,
        max_table_elements: 1,
        ..InstanceLimits::default()
    };
    let rejected_modules = [
        (
            "initial memory",
            "(module (memory 3))",
            LimitFault::MemoryCeiling,
        ),
        (
            "initial table",
            "(module (table 2 funcref))",
            LimitFault::TableCeiling,
        ),
        (
            "second memory",
            "(module (memory 1) (memory 1))",
            LimitFault::MemoryCount,
        ),
        (
            "second table",
            "(module (table 1 funcref) (table 1 funcref))",
            LimitFault::TableCount,
        ),
    ];

    under_both_optimisations(|optimisation| {
        for (label, wat_source, expected_limit) in rejected_modules {
            let result = instantiate_bare(optimisation, wat_source, two_pages_one_element);

            let Err(RuntimeError::LimitExceeded { limit, .. }) = result else {
                panic!("{label}: expected a limit fault");
            };
            assert_eq!(limit, expected_limit, "{label}");
        }

        // A module exactly at both ceilings is accepted.
        let at_the_ceilings = "(module (memory 2) (table 1 funcref))";
        let accepted = instantiate_bare(optimisation, at_the_ceilings, two_pages_one_element);
        assert!(accepted.is_ok(), "{:?}", accepted.err());
    });
}

#[test]
fn memory_growth_past_the_ceiling_is_denied_without_a_fault() {
    under_both_optimisations(|optimisation| {
        let harness = GuestHarness::new(optimisation);
        let two_pages = InstanceLimits {
            max_memory_bytes: 2 * WASM_PAGE_BYTES,
            ..InstanceLimits::default()
        };
        let mut instance = harness.instantiate_with_limits(0, two_pages);

        // `memory.grow` returns the previous size in pages, or -1 when growth is denied.
        let within: i32 = instance
            .call("grow_memory", 1, Deadline::Unbounded)
            .expect("growth to the ceiling is allowed");
        let denied: i32 = instance
            .call("grow_memory", 1, Deadline::Unbounded)
            .expect("denied growth is the guest's result, not a fault");
        assert_eq!((within, denied), (1, -1));

        // The instance keeps running and the granted page is usable.
        let last_word_address = (2 * WASM_PAGE_BYTES - 4) as i32;
        instance
            .call::<(i32, i32), ()>("store", (last_word_address, 7), Deadline::Unbounded)
            .expect("the second page was granted");
        let loaded: i32 = instance
            .call("load", last_word_address, Deadline::Unbounded)
            .expect("the second page was granted");
        assert_eq!(loaded, 7);
    });
}

#[test]
fn table_growth_past_the_ceiling_is_denied_without_a_fault() {
    let growable_table = r#"
        (module
          (table 1 funcref)
          (func (export "grow_table") (param $elements i32) (result i32)
            (table.grow (ref.null func) (local.get $elements))))
    "#;

    under_both_optimisations(|optimisation| {
        let two_elements = InstanceLimits {
            max_table_elements: 2,
            ..InstanceLimits::default()
        };
        let mut instance = instantiate_bare(optimisation, growable_table, two_elements)
            .expect("the initial table fits");

        // `table.grow` returns the previous size, or -1 when growth is denied. The last call
        // shows the denial left the instance running.
        let mut grown = [0; 3];
        for result in &mut grown {
            *result = instance
                .call::<i32, i32>("grow_table", 1, Deadline::Unbounded)
                .expect("denied growth is the guest's result, not a fault");
        }

        assert_eq!(grown, [1, -1, -1]);
    });
}

#[test]
fn limits_are_in_force_while_the_start_function_runs() {
    let growing_start = r#"
        (module
          (import "host" "record" (func $record (param i32)))
          (memory 1)
          (func $start (call $record (memory.grow (i32.const 1))))
          (start $start))
    "#;

    under_both_optimisations(|optimisation| {
        let harness = GuestHarness::new(optimisation);
        let prepared = harness
            .prepare(growing_start)
            .expect("the test host provides the import");
        let one_page = InstanceLimits {
            max_memory_bytes: WASM_PAGE_BYTES,
            ..InstanceLimits::default()
        };

        let instance = prepared
            .instantiate(harness.host_state(0), one_page, Deadline::Unbounded)
            .expect("denied growth doesn't fail the start function");

        // The start function saw its growth denied.
        assert_eq!(instance.host_state().recorded, [-1]);
    });
}

#[test]
fn unbounded_recursion_is_a_stack_limit_fault() {
    under_both_optimisations(|optimisation| {
        let harness = GuestHarness::new(optimisation);
        let mut instance = harness.instantiate(0);

        let result = instance.call::<(), ()>("recurse_forever", (), Deadline::Unbounded);

        let Err(RuntimeError::LimitExceeded { limit, source }) = result else {
            panic!("expected a limit fault");
        };
        assert_eq!(limit, LimitFault::StackExhausted);
        assert!(source.is_some(), "the trap is kept as the source");
    });
}

#[test]
fn guest_running_past_its_deadline_is_interrupted() {
    under_both_optimisations(|optimisation| {
        let harness = GuestHarness::new(optimisation);
        let mut instance = harness.instantiate(0);

        // The guest asks the host to advance the epoch by one tick, then loops forever.
        let result =
            instance.call::<(), ()>("advance_epoch_then_spin", (), Deadline::AfterTicks(1));

        let Err(RuntimeError::LimitExceeded { limit, .. }) = result else {
            panic!("expected a limit fault");
        };
        assert_eq!(limit, LimitFault::DeadlineReached);
    });
}

#[test]
fn deadline_is_measured_from_each_call_and_absent_without_one() {
    under_both_optimisations(|optimisation| {
        let harness = GuestHarness::new(optimisation);
        let mut instance = harness.instantiate(40);

        // Epochs that passed before a call don't count against that call's deadline.
        for _ in 0..3 {
            harness.engine.advance_epoch();
        }
        let bounded: i32 = instance
            .call("add_to_host_value", 1, Deadline::AfterTicks(1))
            .expect("no tick passes during the call");

        harness.engine.advance_epoch();
        let unbounded: i32 = instance
            .call("add_to_host_value", 2, Deadline::Unbounded)
            .expect("a call with no deadline is never interrupted");

        assert_eq!((bounded, unbounded), (41, 42));
    });
}

#[test]
fn start_function_runs_under_the_start_deadline() {
    // The start function records a value, optionally asks the host to advance the epoch by one
    // tick, and optionally loops forever afterwards.
    let returning_start = r#"
        (module
          (import "host" "record" (func $record (param i32)))
          (func $start (call $record (i32.const 1)))
          (start $start))
    "#;
    let spinning_start = r#"
        (module
          (import "host" "advance_epoch" (func $advance_epoch))
          (func $start
            (call $advance_epoch)
            (loop $spin (br $spin)))
          (start $start))
    "#;

    under_both_optimisations(|optimisation| {
        let harness = GuestHarness::new(optimisation);

        // A start function that returns in time completes. An epoch-enabled store begins with
        // a deadline that has already passed, so this fails unless the start deadline was set.
        harness.engine.advance_epoch();
        let returned = harness
            .prepare(returning_start)
            .expect("the test host provides the import")
            .instantiate(
                harness.host_state(0),
                InstanceLimits::default(),
                Deadline::AfterTicks(1),
            )
            .expect("no tick passes while the start function runs");
        assert_eq!(returned.host_state().recorded, [1]);

        // A start function that outlives its deadline is interrupted.
        let interrupted = harness
            .prepare(spinning_start)
            .expect("the test host provides the import")
            .instantiate(
                harness.host_state(0),
                InstanceLimits::default(),
                Deadline::AfterTicks(1),
            );
        let Err(RuntimeError::LimitExceeded { limit, .. }) = interrupted else {
            panic!("expected a limit fault");
        };
        assert_eq!(limit, LimitFault::DeadlineReached);
    });
}
