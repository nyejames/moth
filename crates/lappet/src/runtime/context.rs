//! The embedder's context holder and the instance's per-call slot.
//!
//! The holder owns the value between calls. For one guest entry the runtime moves it into the
//! instance and moves it back before returning, including when the call faults or a host import
//! panics. The slot is empty between calls, so one instance can serve the next entry with
//! different context and cannot leak one call's value into the next.
//!
//! A host function names the type it needs. A missing value or another type is a host fault and
//! the function is not called. The slot holds one value, so a mutable borrow cannot conflict
//! with another borrow of the same context.

use super::error::{ContextFault, HostFault};
use std::any::Any;

/// The embedder-owned holder. It allocates when the value is stored, not on each call.
///
/// The value is `Send` so the same slot can live in a sendable store. A thread-bound instance
/// uses that slot too. Non-`Send` context is not part of this holder.
pub(crate) struct HostContext {
    value: Option<Box<dyn Any + Send>>,
}

impl HostContext {
    pub(crate) fn new<T: Any + Send>(value: T) -> Self {
        Self {
            value: Some(Box::new(value)),
        }
    }

    pub(crate) fn empty() -> Self {
        Self { value: None }
    }

    pub(crate) fn get_mut<T: Any + Send>(&mut self) -> Result<&mut T, ContextFault> {
        match self.value.as_mut() {
            None => Err(ContextFault::Missing),
            Some(value) => value.downcast_mut().ok_or(ContextFault::WrongType),
        }
    }

    pub(super) fn take(&mut self) -> Option<Box<dyn Any + Send>> {
        self.value.take()
    }

    pub(super) fn restore(&mut self, value: Option<Box<dyn Any + Send>>) {
        self.value = value;
    }
}

/// The per-call slot, borrowed for one host import.
pub(crate) struct CallContext<'a> {
    slot: &'a mut Option<Box<dyn Any + Send>>,
}

impl CallContext<'_> {
    pub(super) fn new(slot: &mut Option<Box<dyn Any + Send>>) -> CallContext<'_> {
        CallContext { slot }
    }

    /// Borrows the context as `T`. Failure means the host function must not run.
    pub(crate) fn require<T: Any + Send>(&mut self) -> Result<&mut T, HostFault> {
        match self.slot.as_mut() {
            None => Err(HostFault::Context(ContextFault::Missing)),
            Some(value) => value
                .downcast_mut()
                .ok_or(HostFault::Context(ContextFault::WrongType)),
        }
    }
}
