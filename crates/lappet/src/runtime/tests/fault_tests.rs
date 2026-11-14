//! Fault classification and quarantine.

use super::{GuestHarness, TestHost, under_both_optimisations};
use crate::runtime::instance::{Deadline, Instance};
use crate::runtime::{HostFault, LimitFault, RuntimeError};
use std::error::Error;

/// Which category a faulting export is expected to report.
#[derive(Debug, Clone, Copy)]
enum ExpectedFault {
    GuestTrap,
    Limit(LimitFault),
    HostFailed,
    HostPanicked,
    HostInternal,
}

/// Every faulting export of `guest.wat`, with the deadline its fault needs.
const FAULTING_EXPORTS: [(&str, Deadline, ExpectedFault); 7] = [
    ("trap", Deadline::Unbounded, ExpectedFault::GuestTrap),
    (
        "recurse_forever",
        Deadline::Unbounded,
        ExpectedFault::Limit(LimitFault::StackExhausted),
    ),
    (
        "advance_epoch_then_spin",
        Deadline::AfterTicks(1),
        ExpectedFault::Limit(LimitFault::DeadlineReached),
    ),
    (
        "call_failing_import",
        Deadline::Unbounded,
        ExpectedFault::HostFailed,
    ),
    (
        "call_panicking_import",
        Deadline::Unbounded,
        ExpectedFault::HostPanicked,
    ),
    (
        "call_import_whose_panic_payload_panics_when_dropped",
        Deadline::Unbounded,
        ExpectedFault::HostPanicked,
    ),
    (
        "call_import_with_wrong_result_type",
        Deadline::Unbounded,
        ExpectedFault::HostInternal,
    ),
];

/// Calls a faulting export. Only the one with a result needs another Wasm signature.
fn call_faulting_export(
    instance: &mut Instance<TestHost>,
    export_name: &str,
    deadline: Deadline,
) -> RuntimeError {
    let result = if export_name == "call_import_with_wrong_result_type" {
        instance
            .call::<(), i32>(export_name, (), deadline)
            .map(drop)
    } else {
        instance.call::<(), ()>(export_name, (), deadline)
    };
    result.expect_err("the export always faults")
}

fn is_expected_fault(error: &RuntimeError, expected: ExpectedFault) -> bool {
    match (expected, error) {
        (ExpectedFault::GuestTrap, RuntimeError::GuestTrap { .. }) => true,
        (ExpectedFault::Limit(expected), RuntimeError::LimitExceeded { limit, .. }) => {
            *limit == expected
        }
        (ExpectedFault::HostFailed, RuntimeError::HostFault(HostFault::Failed { .. })) => true,
        (ExpectedFault::HostPanicked, RuntimeError::HostFault(HostFault::Panicked { .. })) => true,
        (ExpectedFault::HostInternal, RuntimeError::HostFault(HostFault::Internal { .. })) => true,
        _ => false,
    }
}

#[test]
fn every_fault_category_is_classified_and_quarantines_its_instance() {
    under_both_optimisations(|optimisation| {
        let harness = GuestHarness::new(optimisation);

        for (export_name, deadline, expected) in FAULTING_EXPORTS {
            let mut instance = harness.instantiate(0);
            let mut sibling = harness.instantiate(0);

            let fault = call_faulting_export(&mut instance, export_name, deadline);
            assert!(
                is_expected_fault(&fault, expected),
                "{export_name}: expected {expected:?}, got {fault:?}"
            );

            // The quarantined instance refuses the call without entering the guest: `record`
            // would otherwise leave its argument in host state.
            let refused = instance.call::<i32, ()>("record", 1, Deadline::Unbounded);
            assert!(
                matches!(refused, Err(RuntimeError::InstanceFaulted)),
                "{export_name}"
            );
            assert!(instance.host_state().recorded.is_empty(), "{export_name}");

            // A sibling instance of the same prepared module is unaffected.
            sibling
                .call::<i32, ()>("record", 1, Deadline::Unbounded)
                .expect("the sibling never faulted");
            assert_eq!(sibling.host_state().recorded, [1], "{export_name}");
        }
    });
}

#[test]
fn quarantined_instance_keeps_its_host_state_until_it_is_dropped_once() {
    under_both_optimisations(|optimisation| {
        let harness = GuestHarness::new(optimisation);
        let mut instance = harness.instantiate(0);
        instance
            .call::<i32, ()>("record", 3, Deadline::Unbounded)
            .expect("the host import has no fault path");

        call_faulting_export(&mut instance, "trap", Deadline::Unbounded);

        // The embedder can still read and drain what the faulted instance owns.
        assert_eq!(instance.host_state().recorded, [3]);
        instance.host_state_mut().recorded.clear();
        assert_eq!(harness.host_states_dropped(), 0);

        drop(instance);
        assert_eq!(harness.host_states_dropped(), 1);
    });
}

#[test]
fn host_faults_carry_what_the_host_reported() {
    under_both_optimisations(|optimisation| {
        let harness = GuestHarness::new(optimisation);

        let mut instance = harness.instantiate(0);
        let failed =
            call_faulting_export(&mut instance, "call_failing_import", Deadline::Unbounded);
        let reported = failed.source().map(ToString::to_string);
        assert_eq!(reported.as_deref(), Some("the disk is full"));

        let mut instance = harness.instantiate(0);
        let panicked =
            call_faulting_export(&mut instance, "call_panicking_import", Deadline::Unbounded);
        let RuntimeError::HostFault(HostFault::Panicked { message }) = panicked else {
            panic!("expected a caught host panic");
        };
        assert_eq!(message, "host import bug");
    });
}
