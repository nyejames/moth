//! Resource limits and execution deadlines.

use super::{BOTH_OPTIMISATIONS, GuestHarness, TestHost, assemble, test_host_imports};
use crate::runtime::imports::HostImports;
use crate::runtime::instance::{Deadline, Instance};
use crate::runtime::{InstanceLimits, LimitFault, NativeOptimisation, RuntimeEngine, RuntimeError};

const WASM_PAGE_BYTES: usize = 65_536;

/// Instantiates an import-free module with no host state under the given limits.
fn instantiate_bare(
    optimisation: NativeOptimisation,
    wat_source: &str,
    limits: InstanceLimits,
) -> Result<Instance<()>, RuntimeError> {
    let engine =
        RuntimeEngine::new(optimisation).expect("the engine policy is supported on a test host");
    let module = engine
        .compile_module(&assemble(wat_source))
        .expect("the module is valid core Wasm");
    let prepared = HostImports::new(&engine)
        .prepare(&module)
        .expect("the module has no imports");

    prepared.instantiate((), limits, Deadline::Unbounded)
}

#[test]
fn module_that_cannot_fit_the_limits_is_rejected_before_it_runs() {
    let two_pages_one_element = InstanceLimits {
        max_memory_bytes: 2 * WASM_PAGE_BYTES,
        max_table_elements: 1,
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

    for optimisation in BOTH_OPTIMISATIONS {
        for (label, wat_source, expected_limit) in rejected_modules {
            let result = instantiate_bare(optimisation, wat_source, two_pages_one_element);

            let Err(RuntimeError::LimitExceeded { limit, .. }) = result else {
                panic!("{optimisation:?}, {label}: expected a limit fault");
            };
            assert_eq!(limit, expected_limit, "{optimisation:?}, {label}");
        }

        // A module exactly at both ceilings is accepted.
        let at_the_ceilings = "(module (memory 2) (table 1 funcref))";
        let accepted = instantiate_bare(optimisation, at_the_ceilings, two_pages_one_element);
        assert!(accepted.is_ok(), "{optimisation:?}: {:?}", accepted.err());
    }
}

#[test]
fn memory_growth_past_the_ceiling_is_denied_without_a_fault() {
    for optimisation in BOTH_OPTIMISATIONS {
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
        assert_eq!((within, denied), (1, -1), "{optimisation:?}");

        // The instance keeps running and the granted page is usable.
        let last_word_address = (2 * WASM_PAGE_BYTES - 4) as i32;
        instance
            .call::<(i32, i32), ()>("store", (last_word_address, 7), Deadline::Unbounded)
            .expect("the second page was granted");
        let loaded: i32 = instance
            .call("load", last_word_address, Deadline::Unbounded)
            .expect("the second page was granted");
        assert_eq!(loaded, 7, "{optimisation:?}");
    }
}

#[test]
fn table_growth_past_the_ceiling_is_denied_without_a_fault() {
    let growable_table = r#"
        (module
          (table 1 funcref)
          (func (export "grow_table") (param $elements i32) (result i32)
            (table.grow (ref.null func) (local.get $elements))))
    "#;

    for optimisation in BOTH_OPTIMISATIONS {
        let two_elements = InstanceLimits {
            max_table_elements: 2,
            ..InstanceLimits::default()
        };
        let mut instance = instantiate_bare(optimisation, growable_table, two_elements)
            .expect("the initial table fits");

        let within: i32 = instance
            .call("grow_table", 1, Deadline::Unbounded)
            .expect("growth to the ceiling is allowed");
        let denied: i32 = instance
            .call("grow_table", 1, Deadline::Unbounded)
            .expect("denied growth is the guest's result, not a fault");

        assert_eq!((within, denied), (1, -1), "{optimisation:?}");
    }
}

#[test]
fn unbounded_recursion_is_a_stack_limit_fault() {
    for optimisation in BOTH_OPTIMISATIONS {
        let harness = GuestHarness::new(optimisation);
        let mut instance = harness.instantiate(0);

        let result = instance.call::<(), ()>("recurse_forever", (), Deadline::Unbounded);

        let Err(RuntimeError::LimitExceeded { limit, source }) = result else {
            panic!("{optimisation:?}: expected a limit fault");
        };
        assert_eq!(limit, LimitFault::StackExhausted, "{optimisation:?}");
        assert!(
            source.is_some(),
            "{optimisation:?}: the trap is kept as the source"
        );
    }
}

#[test]
fn guest_running_past_its_deadline_is_interrupted() {
    for optimisation in BOTH_OPTIMISATIONS {
        let harness = GuestHarness::new(optimisation);
        let mut instance = harness.instantiate(0);

        // The guest asks the host to advance the epoch by one tick, then loops forever.
        let result =
            instance.call::<(), ()>("advance_epoch_then_spin", (), Deadline::AfterTicks(1));

        let Err(RuntimeError::LimitExceeded { limit, .. }) = result else {
            panic!("{optimisation:?}: expected a limit fault");
        };
        assert_eq!(limit, LimitFault::DeadlineReached, "{optimisation:?}");
    }
}

#[test]
fn deadline_is_measured_from_each_call_and_absent_without_one() {
    for optimisation in BOTH_OPTIMISATIONS {
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

        assert_eq!((bounded, unbounded), (41, 42), "{optimisation:?}");
    }
}

#[test]
fn start_function_runs_under_the_start_deadline() {
    let spinning_start = r#"
        (module
          (import "host" "advance_epoch" (func $advance_epoch))
          (func $start
            (call $advance_epoch)
            (loop $spin (br $spin)))
          (start $start))
    "#;

    for optimisation in BOTH_OPTIMISATIONS {
        let harness = GuestHarness::new(optimisation);
        let module = harness
            .engine
            .compile_module(&assemble(spinning_start))
            .expect("the module is valid core Wasm");
        let prepared = test_host_imports(&harness.engine)
            .prepare(&module)
            .expect("the test host provides the import");
        let host_state = TestHost {
            value: 0,
            recorded: Vec::new(),
            engine: harness.engine.clone(),
            drop_count: harness.drop_count.clone(),
        };

        let result = prepared.instantiate(
            host_state,
            InstanceLimits::default(),
            Deadline::AfterTicks(1),
        );

        let Err(RuntimeError::LimitExceeded { limit, .. }) = result else {
            panic!("{optimisation:?}: expected a limit fault");
        };
        assert_eq!(limit, LimitFault::DeadlineReached, "{optimisation:?}");
    }
}
