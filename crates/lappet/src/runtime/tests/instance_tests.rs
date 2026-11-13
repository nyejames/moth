//! Instantiation, host imports, export calls and teardown.

use super::{BOTH_OPTIMISATIONS, GuestHarness, TestHost, assemble, engine, test_host_imports};
use crate::runtime::imports::HostImports;
use crate::runtime::instance::Deadline;
use crate::runtime::{InstanceLimits, LinkError, RuntimeError};
use wasmtime::ValType;

#[test]
fn scalar_values_round_trip_through_an_import_and_an_export() {
    for optimisation in BOTH_OPTIMISATIONS {
        let harness = GuestHarness::new(optimisation);
        let mut instance = harness.instantiate(40);

        let sum: i32 = instance
            .call("add_to_host_value", 2, Deadline::Unbounded)
            .expect("the call has no fault path");

        assert_eq!(sum, 42, "{optimisation:?}");
    }
}

#[test]
fn instances_of_one_prepared_module_share_no_host_state_or_memory() {
    for optimisation in BOTH_OPTIMISATIONS {
        let harness = GuestHarness::new(optimisation);
        let mut first = harness.instantiate(1);
        let mut second = harness.instantiate(2);

        // Each guest writes to the same address of its own memory and its own host state.
        for (instance, value) in [(&mut first, 111), (&mut second, 222)] {
            instance
                .call::<(i32, i32), ()>("store", (64, value), Deadline::Unbounded)
                .expect("the address is inside the first page");
            instance
                .call::<i32, ()>("record", value, Deadline::Unbounded)
                .expect("the host import has no fault path");
        }

        for (instance, host_value, written) in [(&mut first, 1, 111), (&mut second, 2, 222)] {
            let loaded: i32 = instance
                .call("load", 64, Deadline::Unbounded)
                .expect("the address is inside the first page");
            let from_host: i32 = instance
                .call("add_to_host_value", 0, Deadline::Unbounded)
                .expect("the call has no fault path");

            assert_eq!(loaded, written, "{optimisation:?}");
            assert_eq!(from_host, host_value, "{optimisation:?}");
            assert_eq!(
                instance.host_state().recorded,
                [written],
                "{optimisation:?}"
            );
        }
    }
}

#[test]
fn import_the_host_does_not_provide_is_a_missing_import() {
    let wasm_bytes = assemble(include_str!("fixtures/guest.wat"));

    for optimisation in BOTH_OPTIMISATIONS {
        let engine = engine(optimisation);
        let module = engine
            .compile_module(&wasm_bytes)
            .expect("the guest fixture is valid core Wasm");
        let no_imports: HostImports<TestHost> = HostImports::new(&engine);

        let result = no_imports.prepare(&module);

        // Wasmtime reports the first unresolved import in the module's import order.
        let Err(RuntimeError::Link(LinkError::MissingImport { module, name })) = result else {
            panic!("{optimisation:?}: expected a missing import");
        };
        assert_eq!((module.as_str(), name.as_str()), ("host", "read_value"));
    }
}

#[test]
fn import_with_another_physical_signature_is_a_mismatch() {
    let wasm_bytes = assemble(r#"(module (import "host" "read_value" (func (result i64))))"#);

    for optimisation in BOTH_OPTIMISATIONS {
        let engine = engine(optimisation);
        let module = engine
            .compile_module(&wasm_bytes)
            .expect("the module is valid core Wasm");

        // The test host defines `host.read_value` as `() -> i32`.
        let result = test_host_imports(&engine).prepare(&module);

        assert!(
            matches!(
                result,
                Err(RuntimeError::Link(LinkError::ImportMismatch { .. }))
            ),
            "{optimisation:?}"
        );
    }
}

#[test]
fn defining_one_import_name_twice_is_rejected() {
    for optimisation in BOTH_OPTIMISATIONS {
        let engine = engine(optimisation);
        let mut imports = test_host_imports(&engine);

        let result = imports.define(
            "host",
            "read_value",
            [],
            [ValType::I32],
            |_host: &mut TestHost, _arguments, _results| Ok(()),
        );

        let Err(RuntimeError::Link(LinkError::ImportDefinition { module, name, .. })) = result
        else {
            panic!("{optimisation:?}: expected a rejected definition");
        };
        assert_eq!((module.as_str(), name.as_str()), ("host", "read_value"));
    }
}

#[test]
fn bad_export_call_is_a_link_error_and_leaves_the_instance_usable() {
    for optimisation in BOTH_OPTIMISATIONS {
        let harness = GuestHarness::new(optimisation);
        let mut instance = harness.instantiate(5);

        let missing = instance.call::<(), ()>("no_such_export", (), Deadline::Unbounded);
        let Err(RuntimeError::Link(LinkError::MissingExport { name })) = missing else {
            panic!("{optimisation:?}: expected a missing export");
        };
        assert_eq!(name, "no_such_export");

        // A memory export isn't a callable function.
        let not_a_function = instance.call::<(), ()>("memory", (), Deadline::Unbounded);
        assert!(
            matches!(
                not_a_function,
                Err(RuntimeError::Link(LinkError::MissingExport { .. }))
            ),
            "{optimisation:?}"
        );

        // `add_to_host_value` is `(i32) -> i32`.
        let mismatched = instance.call::<i64, i32>("add_to_host_value", 1, Deadline::Unbounded);
        let Err(RuntimeError::Link(LinkError::ExportSignatureMismatch { name, .. })) = mismatched
        else {
            panic!("{optimisation:?}: expected a signature mismatch");
        };
        assert_eq!(name, "add_to_host_value");

        // No guest code ran, so the instance isn't quarantined.
        let sum: i32 = instance
            .call("add_to_host_value", 1, Deadline::Unbounded)
            .expect("link errors don't quarantine");
        assert_eq!(sum, 6, "{optimisation:?}");
    }
}

#[test]
fn trapping_start_function_fails_instantiation_and_drops_host_state_once() {
    let wasm_bytes = assemble("(module (func $start unreachable) (start $start))");

    for optimisation in BOTH_OPTIMISATIONS {
        let harness = GuestHarness::new(optimisation);
        let module = harness
            .engine
            .compile_module(&wasm_bytes)
            .expect("the module is valid core Wasm");
        let prepared = test_host_imports(&harness.engine)
            .prepare(&module)
            .expect("the module has no imports");
        let host_state = TestHost {
            value: 0,
            recorded: Vec::new(),
            engine: harness.engine.clone(),
            drop_count: harness.drop_count.clone(),
        };

        let result =
            prepared.instantiate(host_state, InstanceLimits::default(), Deadline::Unbounded);

        assert!(
            matches!(result, Err(RuntimeError::StartTrap { .. })),
            "{optimisation:?}"
        );
        assert_eq!(harness.host_states_dropped(), 1, "{optimisation:?}");
    }
}

#[test]
fn dropping_an_instance_drops_its_host_state_exactly_once() {
    for optimisation in BOTH_OPTIMISATIONS {
        let harness = GuestHarness::new(optimisation);
        let mut first = harness.instantiate(1);
        let second = harness.instantiate(2);
        first
            .call::<i32, ()>("record", 9, Deadline::Unbounded)
            .expect("the host import has no fault path");
        assert_eq!(harness.host_states_dropped(), 0, "{optimisation:?}");

        drop(first);
        assert_eq!(harness.host_states_dropped(), 1, "{optimisation:?}");

        drop(second);
        assert_eq!(harness.host_states_dropped(), 2, "{optimisation:?}");
    }
}
