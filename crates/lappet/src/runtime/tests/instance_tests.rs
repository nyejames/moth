//! Instantiation, host imports, export calls and teardown.

use super::{GuestHarness, TestHost, engine, prepare, under_both_optimisations};
use crate::runtime::imports::HostImports;
use crate::runtime::instance::Deadline;
use crate::runtime::{InstanceLimits, LinkError, RuntimeError};
use wasmtime::ValType;

#[test]
fn scalar_values_round_trip_through_an_import_and_an_export() {
    under_both_optimisations(|optimisation| {
        let harness = GuestHarness::new(optimisation);
        let mut instance = harness.instantiate(40);

        let sum: i32 = instance
            .call("add_to_host_value", 2, Deadline::Unbounded)
            .expect("the call has no fault path");

        assert_eq!(sum, 42);
    });
}

#[test]
fn instances_of_one_prepared_module_share_no_host_state_or_memory() {
    under_both_optimisations(|optimisation| {
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

            assert_eq!(loaded, written);
            assert_eq!(from_host, host_value);
            assert_eq!(instance.host_state().recorded, [written]);
        }
    });
}

#[test]
fn import_the_host_does_not_provide_is_a_missing_import() {
    under_both_optimisations(|optimisation| {
        let engine = engine(optimisation);
        let no_imports: HostImports<TestHost> = HostImports::new(&engine);

        // The fixture has exactly one import.
        let result = prepare(
            &engine,
            &no_imports,
            include_str!("fixtures/round_trip.wat"),
        );

        let Err(RuntimeError::Link(LinkError::MissingImport { module, name })) = result else {
            panic!("expected a missing import");
        };
        assert_eq!((module.as_str(), name.as_str()), ("host", "read_value"));
    });
}

#[test]
fn an_extra_physical_import_fails_before_guest_start() {
    under_both_optimisations(|optimisation| {
        let harness = GuestHarness::new(optimisation);
        // The start function would trap if preparation succeeded and instantiation ran.
        let result = harness.prepare(
            r#"(module
                (import "host" "read_value" (func (result i32)))
                (import "host" "not_defined" (func))
                (func $start unreachable)
                (start $start))"#,
        );
        let Err(RuntimeError::Link(LinkError::MissingImport { module, name })) = result else {
            panic!("expected the extra import to be rejected before guest start");
        };
        assert_eq!((module.as_str(), name.as_str()), ("host", "not_defined"));
    });
}

#[test]
fn import_with_another_physical_signature_is_a_mismatch() {
    under_both_optimisations(|optimisation| {
        let harness = GuestHarness::new(optimisation);

        // The test host defines `host.read_value` as `() -> i32`.
        let result =
            harness.prepare(r#"(module (import "host" "read_value" (func (result i64))))"#);

        assert!(matches!(
            result,
            Err(RuntimeError::Link(LinkError::ImportMismatch { .. }))
        ));
    });
}

#[test]
fn defining_one_import_name_twice_is_rejected() {
    under_both_optimisations(|optimisation| {
        let mut harness = GuestHarness::new(optimisation);

        let result = harness.imports.define(
            "host",
            "read_value",
            [],
            [ValType::I32],
            |_host, _staging, _limits, _resources, _context, _arguments, _results| Ok(()),
        );

        let Err(RuntimeError::Link(LinkError::ImportDefinition { module, name, .. })) = result
        else {
            panic!("expected a rejected definition");
        };
        assert_eq!((module.as_str(), name.as_str()), ("host", "read_value"));
    });
}

#[test]
fn bad_export_call_is_a_link_error_and_leaves_the_instance_usable() {
    under_both_optimisations(|optimisation| {
        let harness = GuestHarness::new(optimisation);
        let mut instance = harness.instantiate(5);

        let missing = instance.call::<(), ()>("no_such_export", (), Deadline::Unbounded);
        let Err(RuntimeError::Link(LinkError::MissingExport { name })) = missing else {
            panic!("expected a missing export");
        };
        assert_eq!(name, "no_such_export");

        // A memory export isn't a callable function.
        let not_a_function = instance.call::<(), ()>("memory", (), Deadline::Unbounded);
        assert!(matches!(
            not_a_function,
            Err(RuntimeError::Link(LinkError::MissingExport { .. }))
        ));

        // `add_to_host_value` is `(i32) -> i32`.
        let mismatched = instance.call::<i64, i32>("add_to_host_value", 1, Deadline::Unbounded);
        let Err(RuntimeError::Link(LinkError::ExportSignatureMismatch { name, .. })) = mismatched
        else {
            panic!("expected a signature mismatch");
        };
        assert_eq!(name, "add_to_host_value");

        // No guest code ran, so the instance isn't quarantined.
        let sum: i32 = instance
            .call("add_to_host_value", 1, Deadline::Unbounded)
            .expect("link errors don't quarantine");
        assert_eq!(sum, 6);
    });
}

#[test]
fn start_function_runs_at_instantiation_and_can_call_the_host() {
    let recording_start = r#"
        (module
          (import "host" "record" (func $record (param i32)))
          (func $start (call $record (i32.const 7)))
          (start $start))
    "#;

    under_both_optimisations(|optimisation| {
        let harness = GuestHarness::new(optimisation);
        let prepared = harness
            .prepare(recording_start)
            .expect("the test host provides the import");

        let instance = prepared
            .instantiate(
                harness.host_state(0),
                InstanceLimits::default(),
                Deadline::Unbounded,
            )
            .expect("the start function returns");

        assert_eq!(instance.host_state().recorded, [7]);
    });
}

#[test]
fn trapping_start_function_fails_instantiation_and_drops_host_state_once() {
    under_both_optimisations(|optimisation| {
        let harness = GuestHarness::new(optimisation);
        let prepared = harness
            .prepare("(module (func $start unreachable) (start $start))")
            .expect("the module has no imports");

        let result = prepared.instantiate(
            harness.host_state(0),
            InstanceLimits::default(),
            Deadline::Unbounded,
        );

        assert!(matches!(result, Err(RuntimeError::StartTrap { .. })));
        assert_eq!(harness.host_states_dropped(), 1);
    });
}

#[test]
fn dropping_an_instance_drops_its_host_state_exactly_once() {
    under_both_optimisations(|optimisation| {
        let harness = GuestHarness::new(optimisation);
        let mut first = harness.instantiate(1);
        let second = harness.instantiate(2);
        first
            .call::<i32, ()>("record", 9, Deadline::Unbounded)
            .expect("the host import has no fault path");
        assert_eq!(harness.host_states_dropped(), 0);

        drop(first);
        assert_eq!(harness.host_states_dropped(), 1);

        drop(second);
        assert_eq!(harness.host_states_dropped(), 2);
    });
}
