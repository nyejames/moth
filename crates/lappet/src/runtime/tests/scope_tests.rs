//! The host side of the call-scope protocol, under both optimisation levels.
//!
//! The driver opens, reserves, calls the export once, copies the returned span and closes.
//! A trap or a noncanonical status skips close and makes no further guest call.

use super::{assemble, engine, under_both_optimisations};
use crate::runtime::context::HostContext;
use crate::runtime::imports::HostImports;
use crate::runtime::instance::{Deadline, Instance};
use crate::runtime::{BoundaryFault, InstanceLimits, RuntimeError};
use wasmtime::ValType;

struct ScopeHost {
    steps: Vec<i32>,
}

fn instance(optimisation: crate::runtime::NativeOptimisation) -> Instance<ScopeHost> {
    let engine = engine(optimisation);
    let mut imports = HostImports::new(&engine);
    imports
        .define(
            "host",
            "step",
            [ValType::I32],
            [],
            |host: &mut ScopeHost, _, _, _, _, arguments, _| {
                host.steps
                    .push(arguments[0].i32().expect("the import takes one i32"));
                Ok(())
            },
        )
        .expect("the step import is defined once");
    let module = engine
        .compile_module(&assemble(include_str!("fixtures/call_scope.wat")))
        .expect("the fixture is valid core Wasm");
    imports
        .prepare(&module)
        .expect("the fixture imports are provided")
        .instantiate(
            ScopeHost { steps: Vec::new() },
            InstanceLimits::default(),
            Deadline::Unbounded,
        )
        .expect("the fixture has no start function")
}

fn arm(instance: &mut Instance<ScopeHost>, which: i32) {
    instance
        .call::<i32, ()>("arm", which, Deadline::Unbounded)
        .expect("arming a trap does not enter the scope");
}

#[test]
fn a_scoped_call_opens_reserves_calls_once_and_closes() {
    under_both_optimisations(|optimisation| {
        let mut instance = instance(optimisation);
        let mut context = HostContext::empty();
        let result = instance
            .call_scoped("work", b"hi", Deadline::Unbounded, &mut context)
            .expect("every scope step succeeds");
        assert_eq!(result.copied, "hi");
        assert_eq!(instance.host_state().steps, [1, 2, 2, 4, 3]);
        assert!(!instance.has_context());

        instance.host_state_mut().steps.clear();
        let empty = instance
            .call_scoped("work", b"", Deadline::Unbounded, &mut context)
            .expect("an empty argument needs no reservation");
        assert!(empty.copied.is_empty());
        assert_eq!(instance.host_state().steps, [1, 2, 4, 3]);
    });
}

#[test]
fn a_trapping_scope_step_skips_close_and_makes_no_further_guest_call() {
    under_both_optimisations(|optimisation| {
        for (which, expected_steps) in [(1, vec![1]), (2, vec![1, 2]), (4, vec![1, 2, 4])] {
            let mut instance = instance(optimisation);
            arm(&mut instance, which);
            let trapped =
                instance.call_scoped("work", b"", Deadline::Unbounded, &mut HostContext::empty());
            assert!(
                matches!(trapped, Err(RuntimeError::GuestTrap { .. })),
                "{which}"
            );
            assert_eq!(instance.host_state().steps, expected_steps, "{which}");
            assert!(!expected_steps.contains(&3), "{which}");

            let refused =
                instance.call_scoped("work", b"", Deadline::Unbounded, &mut HostContext::empty());
            assert!(
                matches!(refused, Err(RuntimeError::InstanceFaulted)),
                "{which}"
            );
            assert_eq!(instance.host_state().steps, expected_steps, "{which}");
        }
    });
}

#[test]
fn a_noncanonical_result_skips_close() {
    under_both_optimisations(|optimisation| {
        for export_name in ["work_bad_status", "work_bad_span", "work_bad_utf8"] {
            let mut instance = instance(optimisation);
            let rejected = instance.call_scoped(
                export_name,
                b"",
                Deadline::Unbounded,
                &mut HostContext::empty(),
            );
            assert!(
                matches!(
                    rejected,
                    Err(RuntimeError::BoundaryFault(
                        BoundaryFault::NoncanonicalStatus
                    ))
                ),
                "{export_name}"
            );
            assert_eq!(instance.host_state().steps, [1, 2, 4], "{export_name}");
            let refused = instance.call::<i32, ()>("arm", 0, Deadline::Unbounded);
            assert!(
                matches!(refused, Err(RuntimeError::InstanceFaulted)),
                "{export_name}"
            );
            assert_eq!(instance.host_state().steps, [1, 2, 4], "{export_name}");
        }
    });
}

#[test]
fn context_comes_back_when_a_scoped_call_traps() {
    under_both_optimisations(|optimisation| {
        let mut instance = instance(optimisation);
        arm(&mut instance, 1);
        let mut context = HostContext::new(4u32);
        let trapped = instance.call_scoped("work", b"", Deadline::Unbounded, &mut context);
        assert!(matches!(trapped, Err(RuntimeError::GuestTrap { .. })));
        assert!(!instance.has_context());
        assert_eq!(*context.get_mut::<u32>().expect("the value came back"), 4);
    });
}
