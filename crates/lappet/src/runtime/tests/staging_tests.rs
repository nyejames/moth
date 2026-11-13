//! Staging table rules. The table tests need no guest. The fault test uses one only to prove a
//! fault clears the instance's table.

use super::{GuestHarness, under_both_optimisations};
use crate::runtime::instance::Deadline;
use crate::runtime::limits::InstanceLimits;
use crate::runtime::staging::{StagingError, StagingTable};

fn limits(entry_bytes: usize, live_bytes: usize) -> InstanceLimits {
    InstanceLimits {
        max_staged_entry_bytes: entry_bytes,
        max_staged_live_bytes: live_bytes,
        ..InstanceLimits::default()
    }
}

#[test]
fn empty_string_stages_nothing_and_uses_ticket_zero() {
    let mut table = StagingTable::new();
    let limits = InstanceLimits::default();

    assert_eq!(table.stage(b"", &limits), Ok(0));
    assert!(!table.has_live_ticket());
    assert_eq!(
        table.take(0, 0),
        Err(StagingError::UnknownTicket),
        "ticket zero has no entry to take"
    );
}

#[test]
fn tickets_are_nonzero_unique_and_never_reused() {
    let mut table = StagingTable::new();
    let limits = InstanceLimits::default();

    let first = table.stage(b"one", &limits).expect("the entry fits");
    let second = table.stage(b"two", &limits).expect("the entry fits");
    assert_eq!((first, second), (1, 2));

    table.take(first, 3).expect("the length matches");
    table.clear();
    let third = table.stage(b"three", &limits).expect("the entry fits");
    assert_eq!(third, 3, "a taken or cleared ticket is not issued again");
}

#[test]
fn a_failed_take_consumes_nothing() {
    let mut table = StagingTable::new();
    let limits = InstanceLimits::default();
    let ticket = table.stage(b"kept", &limits).expect("the entry fits");

    assert_eq!(table.take(ticket, 2), Err(StagingError::WrongLength));
    assert_eq!(table.take(99, 4), Err(StagingError::UnknownTicket));
    assert_eq!(
        table.take(ticket, 4).expect("the entry is still live"),
        b"kept"
    );
    assert!(!table.has_live_ticket());
}

#[test]
fn entry_and_live_limits_reject_before_staging() {
    let mut table = StagingTable::new();
    let limits = limits(4, 6);

    assert_eq!(
        table.stage(b"abcde", &limits),
        Err(StagingError::EntryTooLarge)
    );
    assert!(!table.has_live_ticket());

    table
        .stage(b"abcd", &limits)
        .expect("the entry is at the ceiling");
    assert_eq!(
        table.stage(b"efg", &limits),
        Err(StagingError::LiveBytesExceeded)
    );
    assert_eq!(
        table
            .take(1, 4)
            .expect("the rejected stage issued no ticket"),
        b"abcd"
    );
}

#[test]
fn clearing_drops_live_entries_and_keeps_issued_tickets() {
    let mut table = StagingTable::new();
    let limits = InstanceLimits::default();
    table.stage(b"one", &limits).expect("the entry fits");
    table.stage(b"two", &limits).expect("the entry fits");

    table.clear();
    assert!(!table.has_live_ticket());
    assert_eq!(table.take(1, 3), Err(StagingError::UnknownTicket));
    assert_eq!(table.stage(b"three", &limits).expect("a new entry fits"), 3);
}

#[test]
fn a_fault_clears_the_instance_table() {
    under_both_optimisations(|optimisation| {
        let harness = GuestHarness::new(optimisation);
        let mut instance = harness.instantiate(0);
        let ticket = instance.stage_text(b"pending").expect("the entry fits");
        assert_eq!(ticket, 1);
        assert!(instance.has_live_staged_text());

        let fault = instance.call::<(), ()>("trap", (), Deadline::Unbounded);
        assert!(fault.is_err(), "the export traps");
        assert!(!instance.has_live_staged_text());
    });
}
