//! Fault classification at the two native boundaries: host imports and guest entry.
//!
//! A host import's failure or panic is turned into a [`HostFault`] before Wasmtime sees it, and
//! travels through the guest's unwinding as that typed value. When a guest entry fails,
//! [`classify_guest_failure`] recovers the typed value or reads Wasmtime's trap code. Nothing
//! here reads message text.

use super::error::{ErrorSource, HostFault, LimitFault, RuntimeError};
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use wasmtime::Trap;

/// Which guest entry failed. A plain trap is reported differently for the start function,
/// because no instance exists afterwards.
#[derive(Debug, Clone, Copy)]
pub(super) enum GuestEntry {
    StartFunction,
    ExportedCall,
}

/// Runs one host import and reports its failure or panic as a typed host fault.
///
/// The panic is caught here so it never unwinds through guest frames as a panic. A host built
/// with `panic = "abort"` aborts the process instead.
pub(super) fn run_host_import(
    host_import: impl FnOnce() -> Result<(), ErrorSource>,
) -> wasmtime::Result<()> {
    // Unwind safety: a caught panic quarantines the instance, so the guest never observes host
    // state the panic left half-updated. The embedder may still inspect that state and owns
    // that risk, as it does for any `catch_unwind` around its own code.
    let outcome = catch_unwind(AssertUnwindSafe(host_import));

    let host_fault = match outcome {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(source)) => HostFault::Failed { source },
        Err(panic_payload) => {
            let message = panic_message(panic_payload.as_ref());
            // A payload's own destructor can panic. Drop it under a second guard and leak
            // whatever that panic carried, so no unwind leaves this function.
            if let Err(destructor_payload) = catch_unwind(AssertUnwindSafe(|| drop(panic_payload)))
            {
                std::mem::forget(destructor_payload);
            }
            HostFault::Panicked { message }
        }
    };
    Err(wasmtime::Error::new(host_fault))
}

/// Classifies why a guest entry failed.
pub(super) fn classify_guest_failure(error: wasmtime::Error, entry: GuestEntry) -> RuntimeError {
    // A host fault raised by `run_host_import` comes back out as the same typed value.
    let error = match error.downcast::<HostFault>() {
        Ok(host_fault) => return RuntimeError::HostFault(host_fault),
        Err(error) => error,
    };
    let error = match error.downcast::<super::error::BoundaryFault>() {
        Ok(boundary_fault) => return RuntimeError::BoundaryFault(boundary_fault),
        Err(error) => error,
    };

    let trap = error.downcast_ref::<Trap>().copied();
    let source = error.into_boxed_dyn_error();

    match (trap, entry) {
        // Exhausted limits are traps in Wasmtime but a distinct category here.
        (Some(Trap::StackOverflow), _) => RuntimeError::LimitExceeded {
            limit: LimitFault::StackExhausted,
            source: Some(source),
        },
        (Some(Trap::Interrupt), _) => RuntimeError::LimitExceeded {
            limit: LimitFault::DeadlineReached,
            source: Some(source),
        },

        // Every other trap code is the guest's own failure.
        (Some(_), GuestEntry::StartFunction) => RuntimeError::StartTrap { source },
        (Some(_), GuestEntry::ExportedCall) => RuntimeError::GuestTrap { source },

        // Not a trap and not a reported host failure: the native side broke an invariant.
        (None, _) => RuntimeError::HostFault(HostFault::Internal { source }),
    }
}

/// The text of a panic payload, when it carries one.
fn panic_message(panic_payload: &(dyn Any + Send)) -> String {
    if let Some(message) = panic_payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = panic_payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "the panic payload was not text".to_string()
    }
}
